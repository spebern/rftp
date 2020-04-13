use base64;
use dirs::home_dir;
use rpassword::prompt_password_stdout;
use std::collections::HashSet;
use std::error::Error;
use std::io::{stdin, stdout, Write};
use std::net::TcpStream;

pub fn create_session(
    destination: &str,
    username: &str,
    port: Option<&str>,
) -> Result<ssh2::Session, Box<dyn Error>> {
    let tcp = if let Some(port) = port {
        let port = port
            .parse::<u16>()
            .map_err(|_| "unable to parse port number")?;
        TcpStream::connect((destination, port))?
    } else {
        TcpStream::connect(destination).unwrap_or(TcpStream::connect((destination, 22))?)
    };
    let port = tcp.peer_addr()?.port();

    let mut session = ssh2::Session::new()?;
    session.set_timeout(10000);
    session.set_compress(true);
    session.set_tcp_stream(tcp);
    session.handshake()?;

    let session = authenticate_host(session, destination, port)?;
    let session = authenticate_session(session, username)?;

    Ok(session)
}

fn authenticate_host(
    session: ssh2::Session,
    destination: &str,
    port: u16,
) -> Result<ssh2::Session, Box<dyn Error>> {
    let mut known_hosts = session.known_hosts()?;
    let known_hosts_path = home_dir()
        .ok_or("unable to find home directory")?
        .join(".ssh/known_hosts");
    known_hosts.read_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
    let (key, key_type) = session.host_key().ok_or("unable to get host key")?;
    match known_hosts.check_port(destination, port, key) {
        ssh2::CheckResult::Match => Ok(session),
        ssh2::CheckResult::NotFound => {
            let fingerprint = session
                .host_key_hash(ssh2::HashType::Sha256)
                .map(|hash| ("SHA256", hash))
                .or_else(|| {
                    session
                        .host_key_hash(ssh2::HashType::Sha1)
                        .map(|hash| ("SHA128", hash))
                })
                .map(|(hash_type, fingerprint)| {
                    format!("{}:{}", hash_type, base64::encode(fingerprint))
                })
                .ok_or("unable to get fingerprint of host")?;

            println!(
                "The host key for {} was not found in {:?}.",
                destination, known_hosts_path
            );
            println!("Fingerprint: {}", fingerprint);
            print!("Would you like to add it (yes/no)? ");
            stdout().flush()?;

            let mut input = String::new();
            stdin().read_line(&mut input)?;
            match input.trim().as_ref() {
                "Y" | "y" | "YES" | "Yes" | "yes" => {
                    known_hosts.add(destination, key, "", key_type.into())?;
                    known_hosts.write_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
                    Ok(session)
                }
                _ => Err(Box::from(format!(
                    "the authenticity of host {} cannot be established",
                    destination
                ))),
            }
        }
        ssh2::CheckResult::Mismatch => {
            eprintln!("####################################################");
            eprintln!("# WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! #");
            eprintln!("####################################################");
            Err(Box::from("possible person in the middle attack"))
        }
        ssh2::CheckResult::Failure => Err(Box::from("failed to check known hosts")),
    }
}

fn authenticate_session(
    session: ssh2::Session,
    username: &str,
) -> Result<ssh2::Session, Box<dyn Error>> {
    for _ in 0..3 {
        if session.authenticated() {
            break;
        }

        let auth_methods: HashSet<&str> = session.auth_methods(username)?.split(",").collect();

        if !session.authenticated() && auth_methods.contains("publickey") {
            session.userauth_agent(username)?;
        }

        if !session.authenticated() && auth_methods.contains("password") {
            let password = prompt_password_stdout("🔐 Password: ")?;
            session.userauth_password(username, &password).ok();
        }

        // if !session.authenticated() && auth_methods.contains("keyboard-interactive") {
        //     // TODO: Need to test.
        //     struct Prompter;
        //     impl ssh2::KeyboardInteractivePrompt for Prompter {
        //         fn prompt(
        //             &mut self,
        //             _username: &str,
        //             instructions: &str,
        //             prompts: &[ssh2::Prompt],
        //         ) -> Vec<String> {
        //             prompts
        //                 .iter()
        //                 .map(|p| {
        //                     println!("{}", instructions);
        //                     if p.echo {
        //                         let mut input = String::new();
        //                         if stdin().read_line(&mut input).is_ok() {
        //                             input
        //                         } else {
        //                             String::new()
        //                         }
        //                     } else {
        //                         prompt_password_stdout(&p.text).unwrap_or_else(|_| String::new())
        //                     }
        //                 })
        //                 .collect()
        //         }
        //     }
        //     let mut prompter = Prompter;
        //     session.userauth_keyboard_interactive(username, &mut prompter)?;
        // }
    }

    if session.authenticated() {
        Ok(session)
    } else {
        Err(Box::from("unable to authenticate session"))
    }
}