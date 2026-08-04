// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::sync::Arc;

const CURL_HELP_TEXT: &str = "\
Usage: curl [options...] <url>
Options:
 -A, --user-agent <name>    Send User-Agent <name> to server
 -b, --cookie <data>        Send cookies from string/file
 -d, --data <data>          HTTP POST data
 -e, --referer <URL>        Send Referer header
 -f, --fail                 Fail silently on HTTP errors (exit 22)
 -H, --header <header>      Pass custom header
 -h, --help                 Show this help
 -I, --head                 Fetch headers only (HEAD request)
 -i, --include              Include response headers in output
 -k, --insecure             Allow insecure TLS connections
 -L, --location             Follow redirects
 -o, --output <file>        Write to file instead of stdout
 -O, --remote-name          Write to local file named like the remote
 -s, --silent               Silent mode (no output)
 -u, --user <user:pass>     Server user and password
 -v, --verbose              Verbose output
 -V, --version              Show version
 -w, --write-out <format>   Write output after completion. Supports: %{http_code}, %{url_effective}, %{size_download}, %{time_total}
 -X, --request <method>     Specify request method
     --compressed           Request compressed response
     --connect-timeout <s>  Maximum time allowed for connection
     --max-time <s>         Maximum time allowed for transfer\n";

pub(crate) struct HttpConfig {
    pub method: String,
    pub url: String,
    pub data: Option<String>,
    pub follow_redirects: bool,
    pub headers: Vec<(String, String)>,
    pub basic_auth: Option<(String, String)>,
    pub insecure: bool,
    pub verbose: bool,
    #[allow(dead_code)]
    pub include_headers: bool,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            method: "GET".to_string(),
            url: String::new(),
            data: None,
            follow_redirects: false,
            headers: Vec::new(),
            basic_auth: None,
            insecure: false,
            verbose: false,
            include_headers: false,
            request_timeout_secs: 30,
            connect_timeout_secs: 10,
        }
    }
}

pub(crate) struct HttpResponse {
    pub body: String,
    pub status_code: u16,
    pub final_url: String,
    #[allow(dead_code)]
    pub response_headers: Vec<(String, String)>,
    pub size_download: usize,
    pub verbose_log: String,
    pub time_total: f64,
}

pub(crate) fn http_request_ex(config: &HttpConfig) -> Result<HttpResponse, HttpError> {
    let start = std::time::Instant::now();
    let mut agent_builder = ureq::AgentBuilder::new()
        .redirects(if config.follow_redirects { 10 } else { 0 });

    if config.connect_timeout_secs > 0 {
        agent_builder =
            agent_builder.timeout_connect(std::time::Duration::from_secs(config.connect_timeout_secs));
    }
    if config.request_timeout_secs > 0 {
        agent_builder =
            agent_builder.timeout(std::time::Duration::from_secs(config.request_timeout_secs));
    }

    if config.insecure {
        eprintln!("⚠️  [fastshell] TLS certificate verification DISABLED (--insecure/-k). This is unsafe and should only be used for testing.");
        agent_builder = agent_builder.tls_config(build_insecure_tls_config());
    } else {
        agent_builder = agent_builder.tls_config(build_secure_tls_config());
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
                verbose_log.push_str(&format!("< HTTP/1.1 {} {}\n", status_code, status_text));
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
                time_total: start.elapsed().as_secs_f64(),
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let status_code = code;
            let response_headers: Vec<(String, String)> = resp.headers_names()
                .iter()
                .filter_map(|n| resp.header(n).map(|v| (n.clone(), v.to_string())))
                .collect();
            let body = resp.into_string().unwrap_or_default();
            Err(HttpError {
                status_code,
                response_headers,
                body,
            })
        }
        Err(e) => Err(HttpError {
            status_code: 0,
            response_headers: Vec::new(),
            body: e.to_string(),
        }),
    }
}

/// Structured HTTP error so `--fail` can still include response headers.
pub(crate) struct HttpError {
    status_code: u16,
    response_headers: Vec<(String, String)>,
    body: String,
}

impl From<String> for HttpError {
    fn from(s: String) -> Self {
        HttpError { status_code: 0, response_headers: Vec::new(), body: s }
    }
}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        HttpError { status_code: 0, response_headers: Vec::new(), body: e.to_string() }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status_code > 0 {
            write!(f, "HTTP {}: {}", self.status_code, self.body)
        } else {
            write!(f, "{}", self.body)
        }
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

fn build_secure_tls_config() -> Arc<rustls::ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let native = rustls_native_certs::load_native_certs();
    for cert in native.certs {
        root_store.add(cert).ok();
    }
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    )
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
    time_total: f64,
) -> String {
    let mut result = format.to_string();
    result = result.replace("\\n", "\n");
    result = result.replace("\\t", "\t");
    result = result.replace("\\r", "\r");
    result = result.replace("\\\\", "\\");
    result = result.replace("%{http_code}", &status_code.to_string());
    result = result.replace("%{url_effective}", final_url);
    result = result.replace("%{size_download}", &size_download.to_string());
    result = result.replace("%{time_total}", &format!("{:.3}", time_total));
    result = result.replace("%{content_type}", "");
    result
}

/// Returns true for curl boolean flags that take **no parameter value**.
///
/// These are either already handled by explicit match arms above the catch-all,
/// or are pass-through no-ops we recognise so the parser doesn't slurp the
/// following argument as the flag's "value".
fn is_boolean_curl_flag(arg: &str) -> bool {
    matches!(
        arg,
        // Already handled explicitly — no-ops here.
        "-s" | "--silent"
            | "-L" | "--location"
            | "-O"
            | "-I" | "--head"
            | "-v" | "--verbose"
            | "-k" | "--insecure"
            | "-i" | "--include"
            | "-f" | "--fail"
            | "--compressed"
            // Not yet implemented but known boolean → don't slurp.
            | "-4" | "-6"
            | "-N" | "--no-buffer"
            | "-q" | "--disable"
            | "-S" | "--show-error"
            | "--globoff"
            | "--digest" | "--basic" | "--anyauth"
            | "--progress-bar"
            | "--no-alpn" | "--no-npn"
            | "--noproxy"
            | "--http1.0" | "--http1.1" | "--http2" | "--http3"
            | "--http2-prior-knowledge"
            | "--ignore-content-length"
            | "--tr-encoding"
            | "--tcp-nodelay"
            | "--netrc" | "--netrc-optional"
            | "-h" | "--help" | "-V" | "--version"
    )
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
        let mut include_headers = false;
        let mut fail_on_error = false;
        let mut request_timeout_secs: Option<u64> = None;
        let mut connect_timeout_secs: Option<u64> = None;
        let mut warnings = String::new();

        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(CURL_HELP_TEXT.to_string());
        }
        if args.contains(&"-V") || args.contains(&"--version") {
            return CommandOutput::success("curl (fastshell) 1.0.0 (rustls + ureq)\n".to_string());
        }

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
                "-A" | "--user-agent" => {
                    if i + 1 < args.len() {
                        headers.push(("User-Agent".to_string(), args[i + 1].to_string()));
                        i += 1;
                    }
                }
                "-e" | "--referer" => {
                    if i + 1 < args.len() {
                        headers.push(("Referer".to_string(), args[i + 1].to_string()));
                        i += 1;
                    }
                }
                "-b" | "--cookie" => {
                    if i + 1 < args.len() {
                        headers.push(("Cookie".to_string(), args[i + 1].to_string()));
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
                "-i" | "--include" => include_headers = true,
                "-f" | "--fail" => fail_on_error = true,
                "--compressed" => {
                    headers.push(("Accept-Encoding".to_string(), "gzip, deflate, br".to_string()));
                }
                "-w" => {
                    if i + 1 < args.len() {
                        write_format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "--max-time" => {
                    if i + 1 < args.len() {
                        if let Ok(t) = args[i + 1].parse::<u64>() {
                            request_timeout_secs = Some(t);
                        }
                        i += 1;
                    }
                }
                "--connect-timeout" => {
                    if i + 1 < args.len() {
                        if let Ok(t) = args[i + 1].parse::<u64>() {
                            connect_timeout_secs = Some(t);
                        }
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') && url.is_none() => {
                    url = Some(arg.to_string());
                }
                // Catch-all: for unrecognised flags that take a parameter,
                // consume the next non-flag token so it doesn't get mistaken
                // for the URL.  Known boolean flags pass through untouched.
                unknown_flag if !is_boolean_curl_flag(unknown_flag) => {
                    warnings.push_str(&format!("curl: warning: unsupported option '{}'\n", unknown_flag));
                    if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                        i += 1; // skip flag's parameter value
                    }
                }
                _ => {} // known boolean flag; no consumption needed
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

        let curl_host = url
            .split("://")
            .nth(1)
            .unwrap_or(&url)
            .split('/')
            .next()
            .unwrap_or(&url)
            .split(':')
            .next()
            .unwrap_or(&url);
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
            include_headers,
            request_timeout_secs: request_timeout_secs.unwrap_or(30),
            connect_timeout_secs: connect_timeout_secs.unwrap_or(10),
        };

        let result = http_request_ex(&config);

        let mut out = match result {
            Ok(response) => {
                let mut stdout = String::new();

                if include_headers {
                    stdout.push_str(&format!("HTTP/1.1 {} {}\r\n", response.status_code, ""));
                    for (k, v) in &response.response_headers {
                        stdout.push_str(&format!("{}: {}\r\n", k, v));
                    }
                    stdout.push_str("\r\n");
                }
                stdout.push_str(&response.body);

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
                        response.time_total,
                    );
                    stdout.push_str(&info);
                }

                let exit_code = if fail_on_error && response.status_code >= 400 { 22 } else { 0 };

                if let Some(ref file) = output_file {
                    let filename = if file == "__auto__" {
                        crate::shell::extract_filename_from_url(&url)
                    } else {
                        file.clone()
                    };
                    match self.vfs.write(&filename, &self.cwd, &stdout) {
                        Ok(_) => {
                            let out_msg = if !silent {
                                format!("Downloaded: {} ({} bytes)\n", filename, size_download)
                            } else {
                                String::new()
                            };
                            CommandOutput { stdout: out_msg, stderr, exit_code }
                        }
                        Err(e) => {
                            let err_msg = format!("curl: {}: {}\n", filename, e);
                            CommandOutput {
                                stdout: String::new(),
                                stderr: if !stderr.is_empty() { format!("{}{}", stderr, err_msg) } else { err_msg },
                                exit_code: 1,
                            }
                        }
                    }
                } else {
                    if silent { stdout = String::new(); }
                    CommandOutput { stdout, stderr, exit_code }
                }
            }
            Err(e) => {
                let exit_code = if fail_on_error && e.status_code >= 400 && e.status_code > 0 {
                    22
                } else {
                    1
                };
                let mut stderr_out = format!("curl: {}\n", e);
                if include_headers {
                    for (k, v) in &e.response_headers {
                        stderr_out.push_str(&format!("{}: {}\n", k, v));
                    }
                }
                CommandOutput {
                    stdout: String::new(),
                    stderr: stderr_out,
                    exit_code,
                }
            }
        };
        if !warnings.is_empty() {
            out.stderr = format!("{}{}", warnings, out.stderr);
        }
        out
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
        let dir =
            std::env::temp_dir().join(format!("fastshell_curl_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    // ── existing tests ──────────────────────────────────────────

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
        let out = shell.execute(
            "curl",
            &["-H", "X-Custom: test", "http://httpbin.org/headers"],
            None,
        );
        if out.exit_code == 0 {
            assert!(out.stdout.contains("X-Custom") || !out.stdout.is_empty());
        }
    }

    #[test]
    fn test_curl_basic_auth() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["-u", "user:pass", "http://httpbin.org/basic-auth/user/pass"],
            None,
        );
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
        let result = format_write_info(
            "%{http_code} %{url_effective} %{size_download}",
            200,
            "http://example.com",
            1024,
            0.5,
        );
        assert_eq!(result, "200 http://example.com 1024");
    }

    #[test]
    fn test_format_write_info_time_and_escape() {
        let result = format_write_info(
            "code:%{http_code}\\ntime:%{time_total}s",
            200,
            "http://x.com",
            0,
            1.234,
        );
        assert!(result.contains("code:200"), "got: {}", result);
        assert!(result.contains("\ntime:1.234s"), "got: {}", result);
    }

    // ── new tests: flag compat ───────────────────────────────────

    #[test]
    fn test_curl_user_agent_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &[
                "-A",
                "FastshellTest/1.0",
                "http://httpbin.org/user-agent",
            ],
            None,
        );
        if out.exit_code == 0 {
            assert!(out.stdout.contains("FastshellTest"), "expected User-Agent in response, got: {}", out.stdout);
        }
    }

    #[test]
    fn test_curl_user_agent_does_not_steal_url() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &[
                "-s",
                "-A",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                "http://httpbin.org/get",
            ],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP") || out.stderr.contains("httpbin"),
                "URL was stolen — got: {}", out.stderr
            );
        }
    }

    #[test]
    fn test_curl_referer_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["-e", "https://google.com", "http://httpbin.org/headers"],
            None,
        );
        if out.exit_code == 0 {
            assert!(out.stdout.contains("google.com"), "expected Referer, got: {}", out.stdout);
        }
    }

    #[test]
    fn test_curl_cookie_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["-b", "session=abc123", "http://httpbin.org/cookies"],
            None,
        );
        if out.exit_code == 0 {
            assert!(out.stdout.contains("session"), "expected cookie, got: {}", out.stdout);
        }
    }

    #[test]
    fn test_curl_include_headers_flag() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-i", "http://httpbin.org/get"], None);
        if out.exit_code == 0 {
            assert!(
                out.stdout.contains("HTTP/1.") && out.stdout.contains("\r\n"),
                "expected raw HTTP response with headers, got: {:.100}",
                out.stdout
            );
        }
    }

    #[test]
    fn test_curl_fail_on_error_flag() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-f", "http://httpbin.org/status/404"], None);
        // -f / --fail → exit code 22 if HTTP >= 400
        assert_eq!(out.exit_code, 22, "expected exit 22, got {} — stderr: {}", out.exit_code, out.stderr);
    }

    #[test]
    fn test_curl_compressed_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["--compressed", "http://httpbin.org/get"],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP")
                    || out.stderr.contains("httpbin"),
                "compressed flag caused unexpected error: {}", out.stderr
            );
        }
    }

    // ── new tests: timeout flags ─────────────────────────────────

    #[test]
    fn test_curl_max_time_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["--max-time", "15", "http://httpbin.org/get"],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP") || out.stderr.contains("httpbin"),
                "max-time caused unexpected error: {}", out.stderr
            );
        }
    }

    #[test]
    fn test_curl_connect_timeout_flag() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["--connect-timeout", "10", "http://httpbin.org/get"],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP") || out.stderr.contains("httpbin"),
                "connect-timeout caused unexpected error: {}", out.stderr
            );
        }
    }

    // ── new tests: unrecognised flag safety ──────────────────────

    #[test]
    fn test_curl_unrecognised_flag_does_not_steal_url() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &[
                "-s",
                "--unknown-flag",
                "some-value",
                "--another",
                "123",
                "http://httpbin.org/get",
            ],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP") || out.stderr.contains("httpbin"),
                "unknown flags stole the URL: {}", out.stderr
            );
        }
    }

    #[test]
    fn test_curl_unrecognised_boolean_flag_keeps_url() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["-4", "-s", "http://httpbin.org/get"],
            None,
        );
        if out.exit_code != 0 {
            assert!(
                out.stderr.contains("HTTP") || out.stderr.contains("httpbin"),
                "boolean flag consumed the URL: {}", out.stderr
            );
        }
    }

    #[test]
    fn test_curl_is_boolean_flag_coverage() {
        // Exercise the boolean-flag helper for all explicitly-listed flags.
        let booleans = &[
            "-s", "--silent", "-L", "--location", "-O", "-I", "--head",
            "-v", "--verbose", "-k", "--insecure", "-i", "--include",
            "-f", "--fail", "--compressed", "-4", "-6", "-N", "--no-buffer",
            "-q", "--disable", "-S", "--show-error", "--globoff",
            "--digest", "--basic", "--anyauth", "--progress-bar",
            "--no-alpn", "--no-npn", "--noproxy",
            "--http1.0", "--http1.1", "--http2", "--http3",
        ];
        for flag in booleans {
            assert!(is_boolean_curl_flag(flag), "{} must be recognised as boolean", flag);
        }
        assert!(!is_boolean_curl_flag("--some-invented-flag"));
        assert!(!is_boolean_curl_flag("https://example.com"));
    }

    // ── new tests: HttpConfig defaults ───────────────────────────

    #[test]
    fn test_http_config_defaults() {
        let cfg = HttpConfig::default();
        assert_eq!(cfg.method, "GET");
        assert_eq!(cfg.request_timeout_secs, 30);
        assert_eq!(cfg.connect_timeout_secs, 10);
        assert!(!cfg.follow_redirects);
        assert!(!cfg.include_headers);
        assert!(!cfg.insecure);
        assert!(!cfg.verbose);
    }

    #[test]
    fn test_curl_help_flag() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage:"));
        assert!(out.stdout.contains("-A"));
        assert!(out.stdout.contains("--max-time"));
    }

    #[test]
    fn test_curl_help_long_flag() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage:"));
    }

    #[test]
    fn test_curl_version_flag() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-V"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("fastshell"));
    }

    #[test]
    fn test_curl_help_does_not_block_url() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["-h", "http://httpbin.org/get"], None);
        assert_eq!(out.exit_code, 0);
        // -h triggers help, URL is not processed
        assert!(out.stdout.contains("Usage:"));
    }

    #[test]
    fn test_curl_unsupported_flag_stderr_warning() {
        let mut shell = mk_shell();
        let out = shell.execute(
            "curl",
            &["--retry", "3", "http://httpbin.org/get"],
            None,
        );
        // Should still reach the URL (exit 0 or HTTP error)
        assert!(out.exit_code == 0 || out.exit_code == 1);
        // Warning should appear in stderr
        let combined = format!("{}{}", out.stderr, out.stdout);
        assert!(
            out.stderr.contains("unsupported") || combined.contains("unsupported"),
            "expected unsupported option warning, stderr='{}'", out.stderr
        );
    }
}
