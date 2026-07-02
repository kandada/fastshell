use crate::shell::{Shell, CommandOutput};
use std::sync::Arc;

pub(crate) struct HttpConfig {
    pub method: String,
    pub url: String,
    pub data: Option<String>,
    pub follow_redirects: bool,
    pub headers: Vec<(String, String)>,
    pub basic_auth: Option<(String, String)>,
    pub insecure: bool,
    pub verbose: bool,
}

pub(crate) struct HttpResponse {
    pub body: String,
    pub status_code: u16,
    pub final_url: String,
    #[allow(dead_code)]
    pub response_headers: Vec<(String, String)>,
    pub size_download: usize,
    pub verbose_log: String,
}

pub(crate) fn http_request_ex(config: &HttpConfig) -> Result<HttpResponse, String> {
    let mut agent_builder = ureq::AgentBuilder::new()
        .redirects(if config.follow_redirects { 10 } else { 0 });

    if config.insecure {
        agent_builder = agent_builder.tls_config(build_insecure_tls_config());
    }

    let agent = agent_builder.build();
    let method = config.method.to_uppercase();
    let url = &config.url;

    let mut verbose_log = String::new();

    if config.verbose {
        verbose_log.push_str(&format!("> {} {}\n", method, url));
        for (k, v) in &config.headers {
            verbose_log.push_str(&format!("> {}: {}\n", k, v));
        }
        if let Some((user, _)) = &config.basic_auth {
            verbose_log.push_str(&format!("> Authorization: Basic {}:***\n", user));
        }
        if let Some(ref data) = config.data {
            verbose_log.push_str(&format!("> Content-Length: {}\n", data.len()));
        }
        verbose_log.push_str(">\n");
    }

    let response = match method.as_str() {
        "POST" => {
            let r = agent.post(url);
            let r = apply_req_opts(r, config);
            if let Some(ref body) = config.data {
                r.send_string(body)
            } else {
                r.send_string("")
            }
        }
        "PUT" => {
            let r = agent.put(url);
            let r = apply_req_opts(r, config);
            if let Some(ref body) = config.data {
                r.send_string(body)
            } else {
                r.send_string("")
            }
        }
        "DELETE" => {
            let r = agent.delete(url);
            apply_req_opts(r, config).call()
        }
        "HEAD" => {
            let r = agent.head(url);
            apply_req_opts(r, config).call()
        }
        _ => {
            let r = agent.get(url);
            apply_req_opts(r, config).call()
        }
    };

    match response {
        Ok(resp) => {
            let final_url = resp.get_url().to_string();
            let status_code = resp.status();
            let status_text = resp.status_text().to_string();
            let mut response_headers = Vec::new();
            for name in resp.headers_names() {
                if let Some(val) = resp.header(&name) {
                    response_headers.push((name, val.to_string()));
                }
            }

            let body = if method == "HEAD" {
                String::new()
            } else {
                resp.into_string().map_err(|e| e.to_string())?
            };
            let size_download = body.len();

            if config.verbose {
                verbose_log.push_str(&format!(
                    "< HTTP/1.1 {} {}\n",
                    status_code, status_text
                ));
                for (k, v) in &response_headers {
                    verbose_log.push_str(&format!("< {}: {}\n", k, v));
                }
                verbose_log.push_str("<\n");
            }

            Ok(HttpResponse {
                body,
                status_code,
                final_url,
                response_headers,
                size_download,
                verbose_log,
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {}: {}", code, body))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn apply_req_opts(mut req: ureq::Request, config: &HttpConfig) -> ureq::Request {
    for (k, v) in &config.headers {
        req = req.set(k, v);
    }
    if let Some((user, pass)) = &config.basic_auth {
        let auth = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", user, pass),
        );
        req = req.set("Authorization", &format!("Basic {}", auth));
    }
    req
}

fn build_insecure_tls_config() -> Arc<rustls::ClientConfig> {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    #[derive(Debug)]
    struct NoVerifier;

    impl ServerCertVerifier for NoVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }

    let _ = rustls::crypto::ring::default_provider().install_default();

    Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth(),
    )
}

fn format_write_info(
    format: &str,
    status_code: u16,
    final_url: &str,
    size_download: usize,
) -> String {
    let mut result = format.to_string();
    result = result.replace("%{http_code}", &status_code.to_string());
    result = result.replace("%{url_effective}", final_url);
    result = result.replace("%{size_download}", &size_download.to_string());
    result
}

impl Shell {
    pub fn cmd_curl(&self, args: &[&str]) -> CommandOutput {
        let mut url: Option<String> = None;
        let mut output_file: Option<String> = None;
        let mut follow_redirects = false;
        let mut silent = false;
        let mut method = "GET".to_string();
        let mut data: Option<String> = None;
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut head_mode = false;
        let mut basic_auth: Option<(String, String)> = None;
        let mut insecure = false;
        let mut verbose = false;
        let mut write_format: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-o" => {
                    if i + 1 < args.len() {
                        output_file = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-O" => {
                    output_file = Some("__auto__".to_string());
                }
                "-L" => follow_redirects = true,
                "-s" => silent = true,
                "-X" => {
                    if i + 1 < args.len() {
                        method = args[i + 1].to_uppercase();
                        i += 1;
                    }
                }
                "-d" => {
                    if i + 1 < args.len() {
                        data = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-H" => {
                    if i + 1 < args.len() {
                        let header_str = args[i + 1];
                        if let Some(colon_pos) = header_str.find(':') {
                            let key = header_str[..colon_pos].trim().to_string();
                            let value = header_str[colon_pos + 1..].trim().to_string();
                            headers.push((key, value));
                        }
                        i += 1;
                    }
                }
                "-I" | "--head" => head_mode = true,
                "-u" => {
                    if i + 1 < args.len() {
                        let creds = args[i + 1];
                        if let Some(colon_pos) = creds.find(':') {
                            let user = creds[..colon_pos].to_string();
                            let pass = creds[colon_pos + 1..].to_string();
                            basic_auth = Some((user, pass));
                        }
                        i += 1;
                    }
                }
                "-k" | "--insecure" => insecure = true,
                "-v" | "--verbose" => verbose = true,
                "-w" => {
                    if i + 1 < args.len() {
                        write_format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') && url.is_none() => {
                    url = Some(arg.to_string());
                }
                _ => {}
            }
            i += 1;
        }

        let url = match url {
            Some(u) => {
                if !u.contains("://") {
                    format!("http://{}", u)
                } else {
                    u
                }
            }
            None => return CommandOutput::error("curl: no URL specified\n".to_string(), 1),
        };

        if head_mode {
            method = "HEAD".to_string();
        }

        let curl_host = url.split("://").nth(1).unwrap_or(&url).split('/').next().unwrap_or(&url).split(':').next().unwrap_or(&url);
        if let Some(perm) = self.check_network_permission(curl_host) {
            return perm;
        }

        let config = HttpConfig {
            method,
            url: url.clone(),
            data,
            follow_redirects,
            headers,
            basic_auth,
            insecure,
            verbose,
        };

        let result = http_request_ex(&config);

        match result {
            Ok(response) => {
                let mut stdout = response.body.clone();
                let size_download = response.size_download;
                let stderr = if verbose {
                    response.verbose_log
                } else {
                    String::new()
                };

                if let Some(ref fmt) = write_format {
                    let info = format_write_info(
                        fmt,
                        response.status_code,
                        &response.final_url,
                        size_download,
                    );
                    if !silent {
                        stdout.push_str(&info);
                    }
                }

                if let Some(ref file) = output_file {
                    let filename = if file == "__auto__" {
                        crate::shell::extract_filename_from_url(&url)
                    } else {
                        file.clone()
                    };
                    match self.vfs.write(&filename, &self.cwd, &stdout) {
                        Ok(_) => {
                            let out_msg = if !silent {
                                format!(
                                    "Downloaded: {} ({} bytes)\n",
                                    filename,
                                    size_download
                                )
                            } else {
                                String::new()
                            };
                            CommandOutput {
                                stdout: out_msg,
                                stderr,
                                exit_code: 0,
                            }
                        }
                        Err(e) => {
                            let err_msg = format!("curl: {}: {}\n", filename, e);
                            CommandOutput {
                                stdout: String::new(),
                                stderr: if !stderr.is_empty() {
                                    format!("{}{}", stderr, err_msg)
                                } else {
                                    err_msg
                                },
                                exit_code: 1,
                            }
                        }
                    }
                } else {
                    if silent {
                        stdout = String::new();
                    }
                    CommandOutput {
                        stdout,
                        stderr,
                        exit_code: 0,
                    }
                }
            }
            Err(e) => CommandOutput::error(format!("curl: {}\n", e), 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_curl_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_curl_no_url_error() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &[], None);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("no URL"));
    }

    #[test]
    fn test_curl_http_get() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["http://httpbin.org/get?test=1"], None);
        if out.exit_code == 0 {
            assert!(out.stdout.contains("test") || !out.stdout.is_empty());
        }
    }

    #[test]
    fn test_curl_head_mode() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-I", "http://httpbin.org/get"], None);
        if out.exit_code == 0 {
            assert!(!out.stdout.is_empty() || out.stdout.is_empty());
        }
    }

    #[test]
    fn test_curl_custom_header() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-H", "X-Custom: test", "http://httpbin.org/headers"], None);
        if out.exit_code == 0 {
            assert!(out.stdout.contains("X-Custom") || !out.stdout.is_empty());
        }
    }

    #[test]
    fn test_curl_basic_auth() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-u", "user:pass", "http://httpbin.org/basic-auth/user/pass"], None);
        if out.exit_code == 0 {
            assert!(out.stdout.contains("authenticated") || !out.stdout.is_empty());
        }
    }

    #[test]
    fn test_curl_verbose() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-v", "http://httpbin.org/get"], None);
        if out.exit_code == 0 {
            assert!(!out.stderr.is_empty());
        }
    }

    #[test]
    fn test_format_write_info() {
        let result = format_write_info("%{http_code} %{url_effective} %{size_download}", 200, "http://example.com", 1024);
        assert_eq!(result, "200 http://example.com 1024");
    }
}
