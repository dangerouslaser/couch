use super::Reply;
use couch_kodi::settings::Settings;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
#[derive(Deserialize)]
struct Setup {
    web_port: u16,
    username: String,
    password: Option<String>,
    #[serde(default)]
    http_control: bool,
}
pub(super) fn route(method: &str, path: &[&str], body: &[u8], file: PathBuf, host: &str) -> Reply {
    let lock = super::connections::lock_for(&file);
    let Ok(_guard) = lock.try_lock() else {
        return Reply::error(503, "Kodi connection is busy");
    };
    let saved = Settings::load(&file).ok().filter(|s| s.host == host);
    match (method, path) {
        ("GET", ["connection"]) => Reply::json(
            200,
            &match saved {
                Some(s) => {
                    json!({"web_port":s.web_port,"username":s.username,"password_set":!s.password.is_empty(),"http_control":s.http_control})
                }
                None => {
                    json!({"web_port":8080,"username":"kodi","password_set":false,"http_control":false})
                }
            },
        ),
        ("PUT", ["connection"]) => {
            let Ok(input) = serde_json::from_slice::<Setup>(body) else {
                return Reply::error(400, "Enter web port, username and password");
            };
            if input.web_port == 0
                || input.username.contains(':')
                || input.username.chars().any(char::is_control)
            {
                return Reply::error(400, "Enter a valid web port and username (without colons)");
            }
            let password = match input.password {
                Some(p) => p,
                None => match saved {
                    Some(s) if s.username == input.username => s.password,
                    _ => return Reply::error(400, "Enter the password for this username"),
                },
            };
            let settings = Settings {
                host: host.into(),
                web_port: input.web_port,
                username: input.username,
                password,
                http_control: input.http_control,
            };
            if couch_control::Kodi::settings(&settings).ping().is_err() {
                return Reply::error(502,"Kodi did not accept the connection. Check its web port, username, password and HTTP control setting.");
            }
            if std::fs::create_dir_all(file.parent().unwrap())
                .and_then(|_| settings.save(&file))
                .is_err()
            {
                return Reply::error(500, "Test passed, but credentials could not be saved");
            }
            Reply::json(
                200,
                &json!({"saved":true,"password_set":!settings.password.is_empty()}),
            )
        }
        _ => Reply::error(404, "Unknown Kodi operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        os::unix::fs::PermissionsExt,
    };
    #[test]
    fn credentials_are_tested_private_and_never_returned_or_replaced_on_failure() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut authorized = false;
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    authorized |= line.trim() == "Authorization: Basic YWxpY2U6c2VjcmV0";
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let body = json!({"jsonrpc":"2.0","id":req["id"],"result":"pong"}).to_string();
                let status = if authorized {
                    "200 OK"
                } else {
                    "401 Unauthorized"
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("couch-kodi-api-{}", std::process::id()));
        let file = dir.join("kodi-connection.json");
        let setup = |password: serde_json::Value| {
            serde_json::to_vec(&json!({"web_port":port,"username":"alice","password":password,"http_control":true})).unwrap()
        };
        assert_eq!(
            route(
                "PUT",
                &["connection"],
                &setup(json!("secret")),
                file.clone(),
                "127.0.0.1"
            )
            .status,
            200
        );
        let before = std::fs::read(&file).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let reply = route("GET", &["connection"], &[], file.clone(), "127.0.0.1");
        assert!(!String::from_utf8(reply.body).unwrap().contains("secret"));
        assert_eq!(
            route(
                "PUT",
                &["connection"],
                &setup(json!("wrong")),
                file.clone(),
                "127.0.0.1"
            )
            .status,
            502
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert_eq!(
            route(
                "PUT",
                &["connection"],
                &setup(serde_json::Value::Null),
                file.clone(),
                "127.0.0.1"
            )
            .status,
            200
        );
        let other = route("GET", &["connection"], &[], file.clone(), "different-host");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&other.body).unwrap()["password_set"],
            false
        );
        thread.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
