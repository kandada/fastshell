// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::sync::Arc;
use ureq::OrAnyStatus;

const CURL_HELP_TEXT: &str = "\
Usage: curl [options...] <url>
 -A, --user-agent <name>        Send User-Agent <name> to server
 -b, --cookie <data|filename>   Send cookies from string/file
 -c, --cookie-jar <filename>    Write cookies to <filename> after operation
 -d, --data <data>              HTTP POST data
     --data-binary <data>       HTTP POST binary data
     --data-raw <data>          HTTP POST data, '@' allowed
     --json <data>              HTTP POST JSON data
 -e, --referer <URL>            Referer URL
 -f, --fail                     Fail fast with no output on HTTP errors
 -G, --get                      Put the post data in the URL and use GET
 -H, --header <header>          Pass custom header(s) to server
 -I, --head                     Show document info only (HEAD request)
 -i, --include                  Include protocol response headers in output
 -k, --insecure                 Allow insecure server connections
 -L, --location                 Follow redirects
 -m, --max-time <seconds>       Maximum time allowed for transfer
 -o, --output <file>            Write to file instead of stdout
 -O, --remote-name              Write output to a file named as the remote file
 -r, --range <range>            Retrieve a byte range from a HTTP server
 -s, --silent                   Silent mode (no error messages)
 -S, --show-error               Show error even when -s is used
 -T, --upload-file <file>       Transfer local FILE to destination
 -u, --user <user:password>     Server user and password
 -v, --verbose                  Make the operation more talkative
 -V, --version                  Show version number and quit
 -w, --write-out <format>       Use output FORMAT after completion
 -X, --request <method>         Specify request method to use
     --compressed               Request compressed response
     --connect-timeout <s>      Maximum time allowed for connection
     --max-redirs <num>         Maximum number of redirects allowed
     --retry <num>              Retry request if transient problems occur
 -h, --help                     This help text
";

const CURL_VERSION_TEXT: &str = "\
curl 8.7.1 (fastshell) libcurl/8.7.1 rustls/0.23 ureq/2.12
Release-Date: 2024-03-27
Protocols: http https
Features: alt-svc AsynchDNS HTTPS-proxy IPv6 Largefile NTLM SSL TLS-SRP UnixSockets
";

/// How the request body supplied via `-d`/`--data*`/`--json` is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DataKind {
    /// `-d`/`--data`/`--data-raw` → `application/x-www-form-urlencoded`.
    #[default]
    Form,
    /// `--data-binary` → no default Content-Type.
    Binary,
    /// `--json` → `application/json`.
    Json,
}

/// Parsed (but not yet resolved) curl command line.
#[derive(Debug, Default, Clone)]
struct CurlOptions {
    url: Option<String>,
    output_file: Option<String>,
    follow_redirects: bool,
    max_redirects: u32,
    silent: bool,
    show_error: bool,
    method: Option<String>,
    head_mode: bool,
    get_mode: bool,
    data_raw: Option<String>,
    data_kind: DataKind,
    form_fields: Vec<(String, String)>,
    upload_file: Option<String>,
    headers: Vec<(String, String)>,
    cookie_raw: Vec<String>,
    basic_auth: Option<(String, String)>,
    insecure: bool,
    verbose: bool,
    write_format: Option<String>,
    include_headers: bool,
    fail_on_error: bool,
    fail_with_body: bool,
    request_timeout: Option<f64>,
    connect_timeout: Option<f64>,
    cookie_jar: Option<String>,
    range: Option<String>,
    continue_at: Option<String>,
    retry: u32,
    show_help: bool,
    show_version: bool,
}

impl CurlOptions {
    fn new() -> Self {
        CurlOptions { max_redirects: 10, ..Default::default() }
    }

    fn set_data(&mut self, raw: &str, kind: DataKind) {
        self.data_raw = Some(raw.to_string());
        self.data_kind = kind;
    }
}

pub(crate) struct HttpConfig {
    pub method: String,
    pub url: String,
    pub data: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub headers: Vec<(String, String)>,
    pub basic_auth: Option<(String, String)>,
    pub insecure: bool,
    pub verbose: bool,
    #[allow(dead_code)]
    pub include_headers: bool,
    pub request_timeout_secs: f64,
    pub connect_timeout_secs: f64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            method: "GET".to_string(),
            url: String::new(),
            data: None,
            content_type: None,
            follow_redirects: false,
            max_redirects: 10,
            headers: Vec::new(),
            basic_auth: None,
            insecure: false,
            verbose: false,
            include_headers: false,
            request_timeout_secs: 30.0,
            connect_timeout_secs: 10.0,
        }
    }
}

pub(crate) struct HttpResponse {
    pub body: String,
    pub status_code: u16,
    pub status_text: String,
    pub final_url: String,
    #[allow(dead_code)]
    pub response_headers: Vec<(String, String)>,
    pub size_download: usize,
    pub verbose_log: String,
    pub time_total: f64,
    pub content_type: String,
    pub remote_ip: String,
    pub remote_port: u16,
}

/// A genuine transport-level error (DNS, connect, TLS, timeout, …), mapped to
/// a curl-compatible exit code. HTTP status codes are *not* errors here.
pub(crate) struct HttpError {
    pub message: String,
    pub curl_exit_code: i32,
}

fn curl_exit_code_for(kind: ureq::ErrorKind, msg: &str) -> i32 {
    match kind {
        ureq::ErrorKind::InvalidUrl => 3,
        ureq::ErrorKind::UnknownScheme => 1,
        ureq::ErrorKind::Dns => 6,
        ureq::ErrorKind::ConnectionFailed => 7,
        ureq::ErrorKind::TooManyRedirects => 47,
        ureq::ErrorKind::Io => {
            let m = msg.to_ascii_lowercase();
            if m.contains("timed out") || m.contains("timeout") {
                28
            } else if m.contains("certificate") || m.contains("tls") || m.contains("handshake") {
                60
            } else {
                7
            }
        }
        _ => 1,
    }
}

pub(crate) fn http_request_ex(config: &HttpConfig) -> Result<HttpResponse, HttpError> {
    let start = std::time::Instant::now();
    let mut agent_builder = ureq::AgentBuilder::new().redirects(if config.follow_redirects {
        config.max_redirects
    } else {
        0
    });

    if config.connect_timeout_secs > 0.0 {
        agent_builder = agent_builder
            .timeout_connect(std::time::Duration::from_secs_f64(config.connect_timeout_secs));
    }
    if config.request_timeout_secs > 0.0 {
        agent_builder =
            agent_builder.timeout(std::time::Duration::from_secs_f64(config.request_timeout_secs));
    }

    if config.insecure {
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

    // `agent.request` supports arbitrary methods (incl. -X CUSTOM), and
    // `or_any_status()` keeps 3xx/4xx/5xx responses as `Ok` so curl semantics
    // (return the body, exit 0 unless `-f`) are preserved.
    let req = apply_req_opts(agent.request(&method, url), config);
    let response = if method == "HEAD" {
        req.call().or_any_status()
    } else {
        match &config.data {
            Some(body) => req.send_bytes(body).or_any_status(),
            None => req.call().or_any_status(),
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

            let content_type = resp
                .header("content-type")
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            let remote = resp.remote_addr();
            let remote_ip = remote.ip().to_string();
            let remote_port = remote.port();

            let body = if method == "HEAD" {
                String::new()
            } else {
                resp.into_string().map_err(|e| HttpError {
                    message: e.to_string(),
                    curl_exit_code: 1,
                })?
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
                status_text,
                final_url,
                response_headers,
                size_download,
                verbose_log,
                time_total: start.elapsed().as_secs_f64(),
                content_type,
                remote_ip,
                remote_port,
            })
        }
        Err(t) => {
            let kind = t.kind();
            let msg = t.to_string();
            Err(HttpError { curl_exit_code: curl_exit_code_for(kind, &msg), message: msg })
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
    if let Some(ct) = &config.content_type {
        if !config.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
            req = req.set("Content-Type", ct);
        }
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

fn var_value(name: &str, resp: &HttpResponse, method: &str) -> String {
    match name {
        "http_code" | "response_code" => resp.status_code.to_string(),
        "http_version" => "1.1".to_string(),
        "url_effective" | "url" => resp.final_url.clone(),
        "size_download" => resp.size_download.to_string(),
        "time_total" => format!("{:.3}", resp.time_total),
        "content_type" => resp.content_type.clone(),
        "method" => method.to_string(),
        "remote_ip" => resp.remote_ip.clone(),
        "remote_port" => resp.remote_port.to_string(),
        "http_connect" | "time_connect" | "time_namelookup" | "time_pretransfer"
        | "time_starttransfer" | "time_appconnect" | "time_redirect" => "0.000".to_string(),
        "num_redirects" => "0".to_string(),
        "redirect_url" => String::new(),
        _ => String::new(),
    }
}

fn format_write_info(format: &str, resp: &HttpResponse, method: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = format.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\\' && i + 1 < n {
            i += 1;
            match chars[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            i += 1;
            continue;
        }
        if c == '%' && i + 1 < n && chars[i + 1] == '{' {
            if let Some(rel) = chars[i + 2..].iter().position(|&x| x == '}') {
                let name: String = chars[i + 2..i + 2 + rel].iter().collect();
                out.push_str(&var_value(&name, resp, method));
                i += 2 + rel + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_value_short_opt(c: char) -> bool {
    matches!(c, 'o' | 'H' | 'd' | 'X' | 'A' | 'b' | 'u' | 'w' | 'e' | 'm' | 'c' | 'r' | 'T'
        | 'F' | 'C' | 'z' | 'Q' | 't' | 'K' | 'y')
}

fn is_bool_short_opt(c: char) -> bool {
    matches!(c, 's' | 'S' | 'L' | 'I' | 'i' | 'v' | 'k' | 'f' | 'O' | 'G' | 'h' | 'V' | '4'
        | '6' | 'N' | 'q' | 'g' | 'j' | 'J' | 'R' | 'n' | 'p' | '#' | '0' | '1' | '2' | '3')
}

impl CurlOptions {
    fn apply_short_value(&mut self, c: char, val: &str) {
        match c {
            'o' => self.output_file = Some(val.to_string()),
            'H' => {
                if let Some(colon) = val.find(':') {
                    let key = val[..colon].trim().to_string();
                    let value = val[colon + 1..].trim().to_string();
                    self.headers.push((key, value));
                } else {
                    self.headers.push((val.to_string(), String::new()));
                }
            }
            'd' => self.set_data(val, DataKind::Form),
            'X' => self.method = Some(val.to_uppercase()),
            'A' => self.headers.push(("User-Agent".to_string(), val.to_string())),
            'b' => self.cookie_raw.push(val.to_string()),
            'u' => {
                if let Some(colon) = val.find(':') {
                    let user = val[..colon].to_string();
                    let pass = val[colon + 1..].to_string();
                    self.basic_auth = Some((user, pass));
                }
            }
            'w' => self.write_format = Some(val.to_string()),
            'e' => self.headers.push(("Referer".to_string(), val.to_string())),
            'm' => self.request_timeout = val.parse::<f64>().ok().filter(|&t| t >= 0.0),
            'c' => self.cookie_jar = Some(val.to_string()),
            'r' => self.range = Some(val.to_string()),
            'T' => self.upload_file = Some(val.to_string()),
            'F' => {
                if let Some(eq) = val.find('=') {
                    let name = val[..eq].to_string();
                    let value = val[eq + 1..].to_string();
                    self.form_fields.push((name, value));
                }
            }
            'C' => self.continue_at = Some(val.to_string()),
            // Recognised but unsupported: silently accepted (no-op) so real
            // curl command lines don't hard-fail.
            'z' | 'Q' | 't' | 'K' | 'y' => {}
            _ => {}
        }
    }

    fn apply_short_bool(&mut self, c: char) {
        match c {
            's' => self.silent = true,
            'S' => self.show_error = true,
            'L' => self.follow_redirects = true,
            'I' => {
                self.head_mode = true;
                self.method = Some("HEAD".to_string());
            }
            'i' => self.include_headers = true,
            'v' => self.verbose = true,
            'k' => self.insecure = true,
            'f' => self.fail_on_error = true,
            'O' => self.output_file = Some("__auto__".to_string()),
            'G' => self.get_mode = true,
            'h' => self.show_help = true,
            'V' => self.show_version = true,
            // Boolean no-ops.
            '4' | '6' | 'N' | 'q' | 'g' | 'j' | 'J' | 'R' | 'n' | 'p' | '#' | '0' | '1' | '2'
            | '3' => {}
            _ => {}
        }
    }

    /// Long options. Returns `Err(message)` for truly unknown options (curl
    /// reports bad options with exit code 2).
    fn apply_long(&mut self, name: &str, val: Option<&str>) -> Result<(), String> {
        // Helper to require a value for options that need one.
        fn need<'a>(name: &str, val: Option<&'a str>) -> Result<&'a str, String> {
            val.ok_or_else(|| format!("option --{}: requires a parameter", name))
        }

        match name {
            "url" => self.url = Some(need(name, val)?.to_string()),
            "output" => self.output_file = Some(need(name, val)?.to_string()),
            "header" => {
                let v = need(name, val)?;
                if let Some(colon) = v.find(':') {
                    let key = v[..colon].trim().to_string();
                    let value = v[colon + 1..].trim().to_string();
                    self.headers.push((key, value));
                } else {
                    self.headers.push((v.to_string(), String::new()));
                }
            }
            "data" | "data-ascii" | "data-raw" | "data-urlencode" => {
                self.set_data(need(name, val)?, DataKind::Form)
            }
            "data-binary" => self.set_data(need(name, val)?, DataKind::Binary),
            "json" => self.set_data(need(name, val)?, DataKind::Json),
            "request" => self.method = Some(need(name, val)?.to_uppercase()),
            "user-agent" => self.headers.push(("User-Agent".to_string(), need(name, val)?.to_string())),
            "cookie" => self.cookie_raw.push(need(name, val)?.to_string()),
            "user" => {
                let v = need(name, val)?;
                if let Some(colon) = v.find(':') {
                    self.basic_auth = Some((v[..colon].to_string(), v[colon + 1..].to_string()));
                }
            }
            "write-out" => self.write_format = Some(need(name, val)?.to_string()),
            "referer" => self.headers.push(("Referer".to_string(), need(name, val)?.to_string())),
            "max-time" => self.request_timeout = need(name, val)?.parse::<f64>().ok(),
            "connect-timeout" => self.connect_timeout = need(name, val)?.parse::<f64>().ok(),
            "cookie-jar" => self.cookie_jar = Some(need(name, val)?.to_string()),
            "range" => self.range = Some(need(name, val)?.to_string()),
            "upload-file" => self.upload_file = Some(need(name, val)?.to_string()),
            "form" => {
                let v = need(name, val)?;
                if let Some(eq) = v.find('=') {
                    self.form_fields.push((v[..eq].to_string(), v[eq + 1..].to_string()));
                }
            }
            "continue-at" => self.continue_at = Some(need(name, val)?.to_string()),
            "max-redirs" => {
                self.max_redirects = need(name, val)?.parse::<u32>().unwrap_or(10);
            }
            "retry" => self.retry = need(name, val)?.parse::<u32>().unwrap_or(0),
            "silent" => self.silent = true,
            "show-error" => self.show_error = true,
            "location" | "location-trusted" => self.follow_redirects = true,
            "head" => {
                self.head_mode = true;
                self.method = Some("HEAD".to_string());
            }
            "include" => self.include_headers = true,
            "verbose" => self.verbose = true,
            "insecure" => self.insecure = true,
            "fail" => self.fail_on_error = true,
            "fail-with-body" => {
                self.fail_on_error = true;
                self.fail_with_body = true;
            }
            "remote-name" => self.output_file = Some("__auto__".to_string()),
            "get" => self.get_mode = true,
            "compressed" => { /* gzip is already the ureq default */ }
            "help" => self.show_help = true,
            "version" => self.show_version = true,
            // ── recognised no-ops (boolean) ─────────────────────────────
            "no-buffer" | "progress-bar" | "globoff" | "digest" | "basic" | "anyauth"
            | "ntlm" | "negotiate" | "netrc" | "netrc-optional" | "http1.0" | "http1.1"
            | "http2" | "http3" | "http2-prior-knowledge" | "no-alpn" | "no-npn"
            | "noproxy" | "ignore-content-length" | "tr-encoding" | "tcp-nodelay"
            | "tcp-fastopen" | "disable" | "disable-epsv" | "disable-eprt" | "ftp-pasv"
            | "crlf" | "no-keepalive" | "ssl-no-revoke" | "cert-status" | "tlsv1"
            | "tlsv1.0" | "tlsv1.1" | "tlsv1.2" | "tlsv1.3" | "sslv2" | "sslv3"
            | "path-as-is" | "suppress-connect-headers" | "proxy-anyauth" | "proxy-basic"
            | "proxy-digest" | "proxy-ntlm" | "proxy-negotiate" | "haproxy-protocol"
            | "hsts" | "no-sessionid" | "ftp-pret" | "ftp-skip-pasv-ip"
            | "post301" | "post302" | "post303" | "sasl-ir" | "use-ascii" | "raw"
            | "no-location-trusted" => {}
            // ── recognised no-ops (take a value) ────────────────────────
            "resolve" | "limit-rate" | "cacert" | "capath" | "cert" | "cert-type"
            | "key" | "key-type" | "pass" | "engine" | "crlfile" | "pinnedpubkey"
            | "random-file" | "egd-file" | "ciphers" | "tls13-ciphers" | "tls-max"
            | "proxy" | "proxy-user" | "proxy-header" | "interface" | "connect-to"
            | "retry-delay" | "retry-max-time" | "speed-limit" | "speed-time" | "stderr"
            | "output-dir" | "trace" | "trace-ascii" | "trace-time" | "dump-header"
            | "config" | "parallel" | "parallel-max" | "rate" | "quote" | "telnet-option"
            | "time-cond" | "proto" | "proto-redir" | "proto-default" | "doh-url"
            | "unix-socket" | "abstract-unix-socket" | "aws-sigv4" | "oauth2-bearer"
            | "service-name" | "delegation" | "dns-interface" | "dns-ipv4-addr"
            | "dns-ipv6-addr" | "dns-servers" | "socks4" | "socks4a" | "socks5"
            | "socks5-hostname" | "socks5-gssapi-service" | "preproxy" | "ftp-account"
            | "ftp-alternative-to-user" | "ftp-method" | "ftp-port" | "mail-from"
            | "mail-rcpt" | "mail-auth" | "pubkey" | "hostpubmd5" | "hostpubsha256" => {
                let _ = val;
            }
            _ => return Err(format!("option --{}: is unknown", name)),
        }
        Ok(())
    }
}

fn parse_curl_args(args: &[&str]) -> Result<CurlOptions, String> {
    let mut o = CurlOptions::new();
    let mut i = 0;

    while i < args.len() {
        let arg = args[i];

        if arg == "--" {
            i += 1;
            while i < args.len() {
                if o.url.is_none() {
                    o.url = Some(args[i].to_string());
                }
                i += 1;
            }
            break;
        }

        if arg == "-" {
            if o.url.is_none() {
                o.url = Some(arg.to_string());
            }
            i += 1;
            continue;
        }

        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline_val) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let mut val = inline_val;
            let takes_value = long_option_takes_value(name);
            if val.is_none() && takes_value {
                i += 1;
                if i < args.len() {
                    val = Some(args[i].to_string());
                }
            }
            o.apply_long(name, val.as_deref())?;
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                if is_value_short_opt(c) {
                    let rest: String = chars[j + 1..].iter().collect();
                    let val = if !rest.is_empty() {
                        Some(rest)
                    } else {
                        i += 1;
                        if i < args.len() {
                            Some(args[i].to_string())
                        } else {
                            None
                        }
                    };
                    match val {
                        Some(v) => o.apply_short_value(c, &v),
                        None => {
                            return Err(format!("option -{}: requires a parameter", c));
                        }
                    }
                    j = chars.len();
                } else if is_bool_short_opt(c) {
                    o.apply_short_bool(c);
                    j += 1;
                } else {
                    return Err(format!("unsupported option: -{}", c));
                }
            }
            i += 1;
            continue;
        }

        if o.url.is_none() {
            o.url = Some(arg.to_string());
        }
        i += 1;
    }

    Ok(o)
}

/// Whether a long option consumes a value (either `--opt=val` or the next arg).
fn long_option_takes_value(name: &str) -> bool {
    matches!(
        name,
        "url" | "output" | "header" | "data" | "data-ascii" | "data-raw" | "data-binary"
            | "data-urlencode" | "json" | "request" | "user-agent" | "cookie" | "user"
            | "write-out" | "referer" | "max-time" | "connect-timeout" | "cookie-jar"
            | "range" | "upload-file" | "form" | "continue-at" | "max-redirs" | "retry"
            | "resolve" | "limit-rate" | "cacert" | "capath" | "cert" | "cert-type" | "key"
            | "key-type" | "pass" | "engine" | "crlfile" | "pinnedpubkey" | "random-file"
            | "egd-file" | "ciphers" | "tls13-ciphers" | "tls-max" | "proxy" | "proxy-user"
            | "proxy-header" | "interface" | "connect-to" | "retry-delay" | "retry-max-time"
            | "speed-limit" | "speed-time" | "stderr" | "output-dir" | "trace" | "trace-ascii"
            | "trace-time" | "dump-header" | "config" | "parallel" | "parallel-max" | "rate"
            | "quote" | "telnet-option" | "time-cond" | "proto" | "proto-redir"
            | "proto-default" | "doh-url" | "unix-socket" | "abstract-unix-socket"
            | "aws-sigv4" | "oauth2-bearer" | "service-name" | "delegation"
            | "dns-interface" | "dns-ipv4-addr" | "dns-ipv6-addr" | "dns-servers" | "socks4"
            | "socks4a" | "socks5" | "socks5-hostname" | "socks5-gssapi-service" | "preproxy"
            | "ftp-account" | "ftp-alternative-to-user" | "ftp-method" | "ftp-port"
            | "mail-from" | "mail-rcpt" | "mail-auth" | "pubkey" | "hostpubmd5"
            | "hostpubsha256"
    )
}

fn resolve_str_arg(shell: &Shell, s: &str) -> String {
    if let Some(path) = s.strip_prefix('@') {
        shell.vfs.read_to_string(path, &shell.cwd).unwrap_or_else(|_| s.to_string())
    } else {
        s.to_string()
    }
}

fn resolve_bytes_arg(shell: &Shell, s: &str) -> Vec<u8> {
    if let Some(path) = s.strip_prefix('@') {
        shell.vfs.read(path, &shell.cwd).unwrap_or_else(|_| s.as_bytes().to_vec())
    } else {
        s.as_bytes().to_vec()
    }
}

fn build_multipart(shell: &Shell, fields: &[(String, String)]) -> (String, Vec<u8>) {
    let boundary = format!("----fastshell-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"", boundary, name).as_bytes(),
        );
        if let Some(path) = value.strip_prefix('@') {
            let fname = path.rsplit('/').next().unwrap_or(path).to_string();
            body.extend_from_slice(
                format!("; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n", fname)
                    .as_bytes(),
            );
            if let Ok(data) = shell.vfs.read(path, &shell.cwd) {
                body.extend_from_slice(&data);
            }
        } else {
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(value.as_bytes());
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    (format!("multipart/form-data; boundary={}", boundary), body)
}

fn build_cookie_header(shell: &Shell, raws: &[String]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for raw in raws {
        if let Some(path) = raw.strip_prefix('@') {
            if let Ok(content) = shell.vfs.read_to_string(path, &shell.cwd) {
                for line in content.lines() {
                    let line = line.trim();
                    if !line.is_empty() && !line.starts_with('#') {
                        parts.push(line.split(';').next().unwrap_or(line).to_string());
                    }
                }
            }
        } else if raw.contains('=') {
            parts.push(raw.clone());
        } else if shell.vfs.exists(raw, &shell.cwd) {
            if let Ok(content) = shell.vfs.read_to_string(raw, &shell.cwd) {
                for line in content.lines() {
                    let line = line.trim();
                    if !line.is_empty() && !line.starts_with('#') {
                        parts.push(line.split(';').next().unwrap_or(line).to_string());
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn write_cookie_jar(shell: &Shell, path: &str, headers: &[(String, String)]) {
    let mut lines = String::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("set-cookie") {
            if let Some(pair) = v.split(';').next() {
                lines.push_str(pair.trim());
                lines.push('\n');
            }
        }
    }
    if !lines.is_empty() {
        let _ = shell.vfs.write(path, &shell.cwd, &lines);
    }
}

impl Shell {
    pub fn cmd_curl(&self, args: &[&str]) -> CommandOutput {
        let opts = match parse_curl_args(args) {
            Ok(o) => o,
            Err(e) => {
                return CommandOutput::error(
                    format!("curl: {}\ncurl: try 'curl --help' for more information\n", e),
                    2,
                )
            }
        };

        if opts.show_help {
            return CommandOutput::success(CURL_HELP_TEXT.to_string());
        }
        if opts.show_version {
            return CommandOutput::success(CURL_VERSION_TEXT.to_string());
        }

        let mut opts = opts;
        let mut warnings = String::new();

        let url = match opts.url {
            Some(u) => {
                if !u.contains("://") {
                    format!("http://{}", u)
                } else {
                    u
                }
            }
            None => {
                return CommandOutput::error(
                    "curl: no URL specified!\ncurl: try 'curl --help' for more information\n"
                        .to_string(),
                    2,
                )
            }
        };

        // Resolve the request body (multipart / upload / data / query).
        let mut data: Option<Vec<u8>> = None;
        let mut content_type: Option<String> = None;
        let mut query_string: Option<String> = None;

        if !opts.form_fields.is_empty() {
            let (ct, body) = build_multipart(self, &opts.form_fields);
            content_type = Some(ct);
            data = Some(body);
        } else if let Some(up) = &opts.upload_file {
            match self.vfs.read(up, &self.cwd) {
                Ok(bytes) => data = Some(bytes),
                Err(e) => {
                    return CommandOutput::error(
                        format!("curl: {}: {}\n", up, e),
                        26, // read error
                    )
                }
            }
        } else if let Some(raw) = &opts.data_raw {
            if opts.get_mode {
                query_string = Some(resolve_str_arg(self, raw));
            } else {
                data = Some(resolve_bytes_arg(self, raw));
                content_type = match opts.data_kind {
                    DataKind::Form => Some("application/x-www-form-urlencoded".to_string()),
                    DataKind::Json => Some("application/json".to_string()),
                    DataKind::Binary => None,
                };
            }
        }

        // Determine the HTTP method (curl precedence: -I/-X, then implied).
        let method = if opts.head_mode {
            "HEAD".to_string()
        } else if let Some(m) = opts.method.take() {
            m
        } else if opts.get_mode {
            "GET".to_string()
        } else if opts.upload_file.is_some() {
            "PUT".to_string()
        } else if data.is_some() {
            "POST".to_string()
        } else {
            "GET".to_string()
        };

        // -G: append data as a query string to the URL.
        let mut final_url = url.clone();
        if let Some(q) = query_string {
            if !q.is_empty() {
                final_url = if final_url.contains('?') {
                    format!("{}&{}", final_url, q)
                } else {
                    format!("{}?{}", final_url, q)
                };
            }
        }

        // Cookies.
        if let Some(cookie) = build_cookie_header(self, &opts.cookie_raw) {
            opts.headers.push(("Cookie".to_string(), cookie));
        }

        // Range / continue-at.
        if let Some(r) = &opts.range {
            opts.headers.push(("Range".to_string(), format!("bytes={}", r)));
        }
        if let Some(c) = &opts.continue_at {
            let offset = if c == "-" {
                let fname = opts
                    .output_file
                    .as_deref()
                    .and_then(|f| if f == "__auto__" {
                        Some(crate::shell::extract_filename_from_url(&final_url))
                    } else {
                        Some(f.to_string())
                    })
                    .and_then(|f| self.vfs.metadata_len(&f, &self.cwd).ok());
                fname.unwrap_or(0)
            } else {
                c.parse::<u64>().unwrap_or(0)
            };
            if offset > 0 {
                opts.headers.push(("Range".to_string(), format!("bytes={}-", offset)));
            }
        }

        let curl_host = final_url
            .split("://")
            .nth(1)
            .unwrap_or(&final_url)
            .split('/')
            .next()
            .unwrap_or(&final_url)
            .split(':')
            .next()
            .unwrap_or(&final_url);
        if let Some(perm) = self.check_network_permission(curl_host) {
            return perm;
        }

        if opts.insecure {
            warnings.push_str(
                "curl: warning: TLS certificate verification disabled (-k). This is unsafe.\n",
            );
        }

        let config = HttpConfig {
            method: method.clone(),
            url: final_url,
            data,
            content_type,
            follow_redirects: opts.follow_redirects,
            max_redirects: opts.max_redirects,
            headers: opts.headers.clone(),
            basic_auth: opts.basic_auth.clone(),
            insecure: opts.insecure,
            verbose: opts.verbose,
            include_headers: opts.include_headers,
            request_timeout_secs: opts.request_timeout.unwrap_or(30.0),
            connect_timeout_secs: opts.connect_timeout.unwrap_or(10.0),
        };

        // --retry: retry transient transport errors up to `retry` times.
        let max_attempts = opts.retry.saturating_add(1).max(1);
        let mut result: Result<HttpResponse, HttpError> =
            Err(HttpError { message: "no attempt".to_string(), curl_exit_code: 1 });
        for attempt in 0..max_attempts {
            if attempt > 0 {
                let backoff = std::time::Duration::from_millis(200 * attempt as u64);
                std::thread::sleep(backoff);
            }
            result = http_request_ex(&config);
            if result.is_ok() || attempt + 1 >= max_attempts {
                break;
            }
        }

        let mut out = match result {
            Ok(response) => {
                if let Some(jar) = &opts.cookie_jar {
                    write_cookie_jar(self, jar, &response.response_headers);
                }

                let mut body_out = String::new();
                if opts.include_headers {
                    body_out.push_str(&format!("HTTP/1.1 {} {}\r\n", response.status_code, response.status_text));
                    for (k, v) in &response.response_headers {
                        body_out.push_str(&format!("{}: {}\r\n", k, v));
                    }
                    body_out.push_str("\r\n");
                }
                body_out.push_str(&response.body);

                let write_out = opts
                    .write_format
                    .as_ref()
                    .map(|fmt| format_write_info(fmt, &response, &method))
                    .unwrap_or_default();

                let stderr = if opts.verbose { response.verbose_log.clone() } else { String::new() };

                let exit_code = if opts.fail_on_error && response.status_code >= 400 { 22 } else { 0 };

                let body_suppressed = opts.fail_on_error && response.status_code >= 400 && !opts.fail_with_body;

                if let Some(file) = opts.output_file.clone() {
                    let filename = if file == "__auto__" {
                        crate::shell::extract_filename_from_url(&config.url)
                    } else {
                        file.clone()
                    };
                    if filename == "/dev/null" {
                        CommandOutput { stdout: write_out, stderr, exit_code }
                    } else if filename == "-" || filename == "/dev/stdout" {
                        let mut stdout = if body_suppressed { String::new() } else { body_out };
                        stdout.push_str(&write_out);
                        CommandOutput { stdout, stderr, exit_code }
                    } else if filename == "/dev/stderr" {
                        let mut stderr = stderr;
                        if !body_suppressed {
                            stderr.push_str(&body_out);
                        }
                        CommandOutput { stdout: write_out, stderr, exit_code }
                    } else {
                        let body_to_write = if body_suppressed { String::new() } else { body_out };
                        match self.vfs.write(&filename, &self.cwd, &body_to_write) {
                            Ok(_) => CommandOutput { stdout: write_out, stderr, exit_code },
                            Err(e) => CommandOutput {
                                stdout: String::new(),
                                stderr: format!("curl: ({}) Failed writing body: {}\n", 23, e),
                                exit_code: 23,
                            },
                        }
                    }
                } else {
                    let mut stdout = if body_suppressed { String::new() } else { body_out };
                    stdout.push_str(&write_out);
                    CommandOutput { stdout, stderr, exit_code }
                }
            }
            Err(e) => {
                let show = !opts.silent || opts.show_error;
                let stderr = if show {
                    format!("curl: ({}) {}\n", e.curl_exit_code, e.message)
                } else {
                    String::new()
                };
                CommandOutput { stdout: String::new(), stderr, exit_code: e.curl_exit_code }
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

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_curl_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Shell::new(Vfs::new(dir).unwrap())
    }

    fn resp() -> HttpResponse {
        HttpResponse {
            body: "hello".to_string(),
            status_code: 200,
            status_text: "OK".to_string(),
            final_url: "http://example.com/x".to_string(),
            response_headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
            size_download: 5,
            verbose_log: String::new(),
            time_total: 0.123,
            content_type: "text/html".to_string(),
            remote_ip: "1.2.3.4".to_string(),
            remote_port: 443,
        }
    }

    // ── write-out formatting ─────────────────────────────────────

    #[test]
    fn test_format_write_info_basic() {
        let r = resp();
        assert_eq!(format_write_info("%{http_code}", &r, "GET"), "200");
        assert_eq!(format_write_info("%{url_effective}", &r, "GET"), "http://example.com/x");
        assert_eq!(format_write_info("%{content_type}", &r, "GET"), "text/html");
        assert_eq!(format_write_info("%{method}", &r, "GET"), "GET");
        assert_eq!(format_write_info("%{remote_ip}:%{remote_port}", &r, "GET"), "1.2.3.4:443");
        assert_eq!(format_write_info("%{size_download}", &r, "GET"), "5");
    }

    #[test]
    fn test_format_write_info_escape_and_unknown() {
        let r = resp();
        let out = format_write_info("code:%{http_code}\\nunknown:[%{nope}]", &r, "GET");
        assert!(out.contains("code:200\nunknown:[]"), "got {}", out);
    }

    // ── argument parsing (no network) ────────────────────────────

    #[test]
    fn test_parse_combined_short_flags() {
        let o = parse_curl_args(&["-sL", "https://example.com"]).unwrap();
        assert!(o.silent);
        assert!(o.follow_redirects);
        assert_eq!(o.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn test_parse_fs_sl() {
        let o = parse_curl_args(&["-fsSL", "https://example.com"]).unwrap();
        assert!(o.silent);
        assert!(o.follow_redirects);
        assert!(o.fail_on_error);
        assert!(o.show_error);
    }

    #[test]
    fn test_parse_m_short_option() {
        let o = parse_curl_args(&["-m", "5", "https://example.com"]).unwrap();
        assert_eq!(o.request_timeout, Some(5.0));
        assert_eq!(o.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn test_parse_attached_value() {
        let o = parse_curl_args(&["-o/dev/null", "https://example.com"]).unwrap();
        assert_eq!(o.output_file.as_deref(), Some("/dev/null"));
    }

    #[test]
    fn test_parse_long_equals() {
        let o = parse_curl_args(&["--max-time=3", "--json", "{\"a\":1}", "https://example.com"])
            .unwrap();
        assert_eq!(o.request_timeout, Some(3.0));
        assert_eq!(o.data_raw.as_deref(), Some("{\"a\":1}"));
        assert_eq!(o.data_kind, DataKind::Json);
    }

    #[test]
    fn test_parse_unknown_option_err() {
        let e = parse_curl_args(&["--totally-bogus", "https://example.com"]).unwrap_err();
        assert!(e.contains("unknown"), "got {}", e);
    }

    #[test]
    fn test_parse_unknown_short_err() {
        let e = parse_curl_args(&["-Z", "https://example.com"]).unwrap_err();
        assert!(e.contains("unsupported"), "got {}", e);
    }

    #[test]
    fn test_parse_get_mode() {
        let o = parse_curl_args(&["-G", "-d", "q=hello", "https://example.com"]).unwrap();
        assert!(o.get_mode);
        assert_eq!(o.data_raw.as_deref(), Some("q=hello"));
    }

    #[test]
    fn test_parse_head_version_help() {
        assert!(parse_curl_args(&["-V"]).unwrap().show_version);
        assert!(parse_curl_args(&["--help"]).unwrap().show_help);
        let o = parse_curl_args(&["-I", "https://example.com"]).unwrap();
        assert!(o.head_mode);
        assert_eq!(o.method.as_deref(), Some("HEAD"));
    }

    // ── end-to-end behaviour (offline: 127.0.0.1:1 refuses connection) ──

    #[test]
    fn test_curl_silent_suppresses_error() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["-s", "http://127.0.0.1:1/"]);
        assert_eq!(out.exit_code, 7, "stderr={}", out.stderr);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty(), "silent should suppress error, got {}", out.stderr);
    }

    #[test]
    fn test_curl_show_error() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["-sS", "http://127.0.0.1:1/"]);
        assert_eq!(out.exit_code, 7);
        assert!(!out.stderr.is_empty(), "show-error should print the error");
    }

    #[test]
    fn test_curl_combined_flags_no_unsupported_warning() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["-sL", "-m", "3", "http://127.0.0.1:1/"]);
        assert_eq!(out.exit_code, 7, "stderr={}", out.stderr);
        assert!(!out.stderr.contains("unsupported"), "got {}", out.stderr);
    }

    #[test]
    fn test_curl_unknown_option_exit_2() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["--bogus-flag", "http://example.com"]);
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("unknown"));
    }

    #[test]
    fn test_curl_no_url_exit_2() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&[]);
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("no URL"));
    }

    #[test]
    fn test_curl_version() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["-V"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("curl"));
        assert!(out.stdout.contains("Protocols"));
    }

    #[test]
    fn test_curl_help() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["--help"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage:"));
        assert!(out.stdout.contains("--max-time"));
        assert!(out.stdout.contains("--json"));
    }

    #[test]
    fn test_curl_json_data_parsed_ok() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["--json", "{}", "http://127.0.0.1:1/"]);
        // POST JSON; connection refused → exit 7 (parsing succeeded, no option error).
        assert_eq!(out.exit_code, 7, "stderr={}", out.stderr);
    }

    #[test]
    fn test_curl_upload_file_missing() {
        let shell = mk_shell();
        let out = shell.cmd_curl(&["-T", "nonexistent.bin", "http://127.0.0.1:1/"]);
        assert_eq!(out.exit_code, 26, "missing upload file should be a read error (26)");
    }
}

