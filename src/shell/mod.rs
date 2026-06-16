use crate::vfs::Vfs;
use std::fs;
use std::process::Command as ProcessCommand;

pub mod commands;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CommandOutput {
    pub fn success(stdout: String) -> Self {
        CommandOutput {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    pub fn error(stderr: String, exit_code: i32) -> Self {
        CommandOutput {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }
}

pub struct Shell {
    pub vfs: Vfs,
    pub cwd: String,
}

impl Shell {
    pub fn new(vfs: Vfs) -> Self {
        Shell {
            vfs,
            cwd: "/".to_string(),
        }
    }

    pub fn execute(&mut self, command: &str, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        match command {
            "ls" => self.cmd_ls(args),
            "cd" => self.cmd_cd(args),
            "pwd" => self.cmd_pwd(args),
            "mkdir" => self.cmd_mkdir(args),
            "rm" => self.cmd_rm(args),
            "cp" => self.cmd_cp(args),
            "mv" => self.cmd_mv(args),
            "cat" => self.cmd_cat(args, stdin),
            "find" => self.cmd_find(args),
            "grep" => self.cmd_grep(args, stdin),
            "echo" => self.cmd_echo(args),
            "touch" => self.cmd_touch(args),
            "chmod" => self.cmd_chmod(args),
            "kill" => self.cmd_kill(args),
            "ps" => self.cmd_ps(args),
            "curl" => self.cmd_curl(args),
            "wget" => self.cmd_wget(args),
            "gzip" => self.cmd_gzip(args),
            "gunzip" => self.cmd_gunzip(args),
            "tar" => self.cmd_tar(args),
            "ping" => self.cmd_ping(args),
            "ssh" => self.cmd_ssh(args),
            #[cfg(feature = "git")]
            "git" => self.cmd_git(args),
            #[cfg(not(feature = "git"))]
            "git" => CommandOutput::error(
                "git: not compiled in (enable 'git' feature)\n".to_string(),
                127,
            ),
            "head" => self.cmd_head(args, stdin),
            "tail" => self.cmd_tail(args, stdin),
            "wc" => self.cmd_wc(args, stdin),
            "diff" => self.cmd_diff(args),
            "sed" => self.cmd_sed(args, stdin),
            "sort" => self.cmd_sort(args, stdin),
            "uniq" => self.cmd_uniq(args, stdin),
            "tee" => self.cmd_tee(args, stdin),
            "xargs" => self.cmd_xargs(args, stdin),
            "which" => self.cmd_which(args),
            "cut" => self.cmd_cut(args, stdin),
            "awk" => self.cmd_awk(args, stdin),
            "tr" => self.cmd_tr(args, stdin),
            "sleep" => self.cmd_sleep(args),
            "date" => self.cmd_date(args),
            "true" => self.cmd_true(args),
            "false" => self.cmd_false_(args),
            "[" | "test" => self.cmd_test(args),
            "base64" => self.cmd_base64(args, stdin),
            "sha256sum" => self.cmd_sha256sum(args, stdin),
            "sha512sum" => self.cmd_sha512sum(args, stdin),
            "md5sum" | "md5" => self.cmd_md5sum(args, stdin),
            "du" => self.cmd_du(args),
            "df" => self.cmd_df(args),
            "stat" => self.cmd_stat(args),
            "jq" => self.cmd_jq(args, stdin),
            "env" => self.cmd_env(args),
            "printenv" => self.cmd_printenv(args),
            "printf" => self.cmd_printf(args, stdin),
            "basename" => self.cmd_basename(args),
            "dirname" => self.cmd_dirname(args),
            "realpath" => self.cmd_realpath(args),
            "file" => self.cmd_file(args, stdin),
            "column" => self.cmd_column(args, stdin),
            "seq" => self.cmd_seq(args),
            "zip" => self.cmd_zip(args),
            "unzip" => self.cmd_unzip(args),
            "shuf" => self.cmd_shuf(args, stdin),
            "uuidgen" => self.cmd_uuidgen(args),
            "rev" => self.cmd_rev(args, stdin),
            "split" => self.cmd_split(args, stdin),
            "comm" => self.cmd_comm(args, stdin),
            "xxd" => self.cmd_xxd(args, stdin),
            "expr" => self.cmd_expr(args),
            "uname" => self.cmd_uname(args),
            "hostname" => self.cmd_hostname(args),
            "whoami" => self.cmd_whoami(args),
            "id" => self.cmd_id(args),
            "pgrep" => self.cmd_pgrep(args),
            "pkill" => self.cmd_pkill(args),
            "paste" => self.cmd_paste(args, stdin),
            "timeout" => self.cmd_timeout(args),
            "ln" => self.cmd_ln(args),
            "readlink" => self.cmd_readlink(args),
            "rmdir" => self.cmd_rmdir(args),
            "mktemp" => self.cmd_mktemp(args),
            "tac" => self.cmd_tac(args, stdin),
            "nl" => self.cmd_nl(args, stdin),
            "truncate" => self.cmd_truncate(args),
            "cmp" => self.cmd_cmp(args),
            "strings" => self.cmd_strings(args, stdin),
            "fold" => self.cmd_fold(args, stdin),
            "expand" => self.cmd_expand(args, stdin),
            "unexpand" => self.cmd_unexpand(args, stdin),
            "yes" => self.cmd_yes(args),
            "sha1sum" => self.cmd_sha1sum(args, stdin),
            "sum" => self.cmd_sum(args, stdin),
            "pidof" => self.cmd_pidof(args),
            "nproc" => self.cmd_nproc(args),
            "tty" => self.cmd_tty(args),
            "clear" => self.cmd_clear(args),
            "sync" => self.cmd_sync(args),
            "nice" => self.cmd_nice(args),
            "chown" => self.cmd_chown(args),
            "chgrp" => self.cmd_chgrp(args),
            "groups" => self.cmd_groups(args),
            "dd" => self.cmd_dd(args),
            "od" => self.cmd_od(args, stdin),
            "uptime" => self.cmd_uptime(args),
            "free" => self.cmd_free(args),
            "nslookup" => self.cmd_nslookup(args),
            "bzip2" => self.cmd_bzip2(args),
            "bunzip2" => self.cmd_bunzip2(args),
            "xz" => self.cmd_xz(args),
            "unxz" => self.cmd_unxz(args),
            "zcat" => self.cmd_zcat(args),
            "dos2unix" => self.cmd_dos2unix(args),
            "unix2dos" => self.cmd_unix2dos(args),
            "cal" => self.cmd_cal(args),
            "logger" => self.cmd_logger(args),
            "dmesg" => self.cmd_dmesg(args),
            "pstree" => self.cmd_pstree(args),
            "killall" => self.cmd_killall(args),
            "watch" => self.cmd_watch(args),
            "logname" => self.cmd_logname(args),
            "who" => self.cmd_who(args),
            "reset" => self.cmd_reset(args),
            "hexdump" => self.cmd_hexdump(args, stdin),
            "sha3sum" => self.cmd_sha3sum(args, stdin),
            "tsort" => self.cmd_tsort(args, stdin),
            "renice" => self.cmd_renice(args),
            "nohup" => self.cmd_nohup(args),
            "chroot" => self.cmd_chroot(args),
            "mkfifo" => self.cmd_mkfifo(args),
            "install" => self.cmd_install(args),
            "shred" => self.cmd_shred(args),
            "fallocate" => self.cmd_fallocate(args),
            "telnet" => self.cmd_telnet(args),
            "traceroute" => self.cmd_traceroute(args),
            "ifconfig" => self.cmd_ifconfig(args),
            "netstat" => self.cmd_netstat(args),
            "nc" => self.cmd_nc(args),
            "patch" => self.cmd_patch(args, stdin),
            "mknod" => self.cmd_mknod(args),
            "mount" => self.cmd_mount(args),
            "umount" => self.cmd_umount(args),
            "whois" => self.cmd_whois(args),
            "hostid" => self.cmd_hostid(args),
            "bc" => self.cmd_bc(args, stdin),
            "iostat" => self.cmd_iostat(args),
            "vmstat" => self.cmd_vmstat(args),
            "lsblk" => self.cmd_lsblk(args),
            "lsof" => self.cmd_lsof(args),
            "dig" => self.cmd_dig(args),
            "rsync" => self.cmd_rsync(args),
            "hdparm" => self.cmd_hdparm(args),
            "smartctl" => self.cmd_smartctl(args),
            "blkid" => self.cmd_blkid(args),
            "lsusb" => self.cmd_lsusb(args),
            "ss" => self.cmd_ss(args),
            "ip" => self.cmd_ip(args),
            "ethtool" => self.cmd_ethtool(args),
            "service" => self.cmd_service(args),
            "showmount" => self.cmd_showmount(args),
            _ => self.run_subprocess(command, args),
        }
    }

    // ── command implementations moved to commands/*.rs ──

    fn run_subprocess(&self, command: &str, args: &[&str]) -> CommandOutput {
        let vfs_root = self.vfs.root().to_path_buf();
        let cwd = if self.cwd == "/" {
            vfs_root.clone()
        } else {
            vfs_root.join(self.cwd.trim_start_matches('/'))
        };

        match ProcessCommand::new(command)
            .args(args)
            .current_dir(&cwd)
            .output()
        {
            Ok(out) => CommandOutput {
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                exit_code: out.status.code().unwrap_or(-1),
            },
            Err(e) => CommandOutput {
                stdout: String::new(),
                stderr: format!("{}: command not found ({})\n", command, e),
                exit_code: 127,
            },
        }
    }
}

pub(crate) fn mode_string(metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o777;
        format!(
            "{}{}{}{}{}{}{}{}{}",
            if mode & 0o400 != 0 { 'r' } else { '-' },
            if mode & 0o200 != 0 { 'w' } else { '-' },
            if mode & 0o100 != 0 { 'x' } else { '-' },
            if mode & 0o040 != 0 { 'r' } else { '-' },
            if mode & 0o020 != 0 { 'w' } else { '-' },
            if mode & 0o010 != 0 { 'x' } else { '-' },
            if mode & 0o004 != 0 { 'r' } else { '-' },
            if mode & 0o002 != 0 { 'w' } else { '-' },
            if mode & 0o001 != 0 { 'x' } else { '-' },
        )
    }
    #[cfg(not(unix))]
    {
        "rw-r--r--".to_string()
    }
}

pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}", bytes)
    } else {
        format!("{:.1}{}", size, UNITS[unit])
    }
}

pub(crate) fn format_unix_time(secs: u64) -> String {
    let secs = secs as i64;
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = civil_from_days(days_since_epoch as i32);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

pub(crate) fn civil_from_days(days: i32) -> (i32, i32, i32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i32, d as i32)
}

pub(crate) fn http_request(
    method: &str,
    url: &str,
    data: Option<&str>,
    follow_redirects: bool,
) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(if follow_redirects { 10 } else { 0 })
        .build();

    let response = match method {
        "POST" => {
            let req = agent.post(url);
            if let Some(body) = data {
                req.send_string(body)
            } else {
                req.send_string("")
            }
        }
        "PUT" => {
            let req = agent.put(url);
            if let Some(body) = data {
                req.send_string(body)
            } else {
                req.send_string("")
            }
        }
        _ => agent.get(url).call(),
    };

    match response {
        Ok(resp) => {
            let body = resp.into_string().map_err(|e| e.to_string())?;
            Ok(body)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {}: {}", code, body))
        }
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn extract_filename_from_url(url: &str) -> String {
    let path = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('/')
        .last()
        .unwrap_or("index.html");
    if path.is_empty() {
        "index.html".to_string()
    } else {
        path.to_string()
    }
}

#[cfg(feature = "git")]
pub(crate) fn extract_repo_name(url: &str) -> String {
    let name = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split('/')
        .last()
        .unwrap_or("repo");
    if name.is_empty() { "repo" } else { name }.to_string()
}

#[cfg(feature = "git")]
pub(crate) fn status_code(status: git2::Status, staged: bool) -> char {
    if staged {
        if status.contains(git2::Status::INDEX_NEW)
            || status.contains(git2::Status::WT_NEW)
        {
            'A'
        } else if status.contains(git2::Status::INDEX_MODIFIED) {
            'M'
        } else if status.contains(git2::Status::INDEX_DELETED) {
            'D'
        } else if status.contains(git2::Status::INDEX_RENAMED) {
            'R'
        } else {
            ' '
        }
    } else {
        if status.contains(git2::Status::WT_MODIFIED) {
            'M'
        } else if status.contains(git2::Status::WT_DELETED) {
            'D'
        } else {
            ' '
        }
    }
}

pub(crate) struct ProcInfo {
    pid: u32,
    ppid: u32,
    comm: String,
    rss: u64,
    cpu_pct: f64,
}

pub(crate) fn list_processes() -> Result<Vec<ProcInfo>, String> {
    #[cfg(target_os = "linux")]
    {
        list_processes_linux()
    }
    #[cfg(target_os = "macos")]
    {
        list_processes_macos()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("ps: not supported on this platform".to_string())
    }
}

#[cfg(target_os = "linux")]
fn list_processes_linux() -> Result<Vec<ProcInfo>, String> {
    let mut procs = Vec::new();
    let proc_dir = std::path::Path::new("/proc");
    if !proc_dir.is_dir() {
        return Err("ps: /proc not available".to_string());
    }

    let total_cpu = read_proc_stat_total_cpu()?;

    for entry in fs::read_dir(proc_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            let stat_path = entry.path().join("stat");
            if let Ok(stat) = fs::read_to_string(&stat_path) {
                if let Some(proc) = parse_proc_stat(pid, &stat, total_cpu) {
                    if let Some(cmdline) = read_proc_cmdline(pid) {
                        let comm = if cmdline.is_empty() {
                            proc.comm
                        } else {
                            cmdline
                        };
                        procs.push(ProcInfo { comm, ..proc });
                    } else {
                        procs.push(proc);
                    }
                }
            }
        }
    }
    Ok(procs)
}

#[cfg(target_os = "linux")]
fn read_proc_stat_total_cpu() -> Result<u64, String> {
    let stat = fs::read_to_string("/proc/stat").map_err(|e| e.to_string())?;
    for line in stat.lines() {
        if line.starts_with("cpu ") {
            let sum: u64 = line
                .split_whitespace()
                .skip(1)
                .filter_map(|s| s.parse::<u64>().ok())
                .sum();
            return Ok(sum);
        }
    }
    Ok(1)
}

#[cfg(target_os = "linux")]
fn parse_proc_stat(pid: u32, stat: &str, _total_cpu: u64) -> Option<ProcInfo> {
    let parts: Vec<&str> = stat.split_whitespace().collect();
    if parts.len() < 25 {
        return None;
    }

    let comm = parts[1].trim_matches('(').trim_matches(')').to_string();
    let state = parts[2].chars().next()?;
    let ppid: u32 = parts[3].parse().ok()?;
    let utime: u64 = parts[13].parse().ok()?;
    let stime: u64 = parts[14].parse().ok()?;
    let cutime: u64 = parts[15].parse().ok()?;
    let cstime: u64 = parts[16].parse().ok()?;
    let rss: u64 = parts[23].parse().ok()?;
    let cpu_total = utime + stime + cutime + cstime;

    let cpu_pct = if _total_cpu > 0 {
        (cpu_total as f64 / _total_cpu as f64) * 100.0
    } else {
        0.0
    };

    Some(ProcInfo {
        pid: pid as u32,
        ppid,
        comm,
        rss: rss * 4,
        cpu_pct,
    })
}

#[cfg(target_os = "linux")]
fn read_proc_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{}/cmdline", pid);
    let data = fs::read(&path).ok()?;
    if data.is_empty() {
        return None;
    }
    let cmdline: String = data
        .split(|&b| b == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .collect::<Vec<&str>>()
        .join(" ");
    if cmdline.is_empty() {
        None
    } else {
        Some(cmdline)
    }
}

#[cfg(target_os = "macos")]
fn list_processes_macos() -> Result<Vec<ProcInfo>, String> {
    let mut procs = Vec::new();

    let mut pids: Vec<libc::c_int> = vec![0; 4096];
    let bufsize = (pids.len() * std::mem::size_of::<libc::c_int>()) as i32;

    let used = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr() as *mut libc::c_void,
            bufsize,
        )
    };

    if used <= 0 {
        return Err("ps: proc_listallpids failed".to_string());
    }

    let count = used as usize / std::mem::size_of::<libc::c_int>();
    if count > pids.len() {
        return Err("ps: too many processes".to_string());
    }

    for &pid in &pids[..count] {
        if pid == 0 {
            continue;
        }
        if let Some(info) = get_proc_info_macos(pid) {
            procs.push(info);
        }
    }

    Ok(procs)
}

#[cfg(target_os = "macos")]
fn get_proc_info_macos(pid: libc::c_int) -> Option<ProcInfo> {
    unsafe {
        const PROC_PIDTASKINFO: libc::c_int = 4;
        const MAXCOMLEN: usize = 16;

        #[repr(C)]
        struct ProcTaskInfo {
            pti_virtual_size: u64,
            pti_resident_size: u64,
            pti_total_user: u64,
            pti_total_system: u64,
            pti_threads_user: u64,
            pti_threads_system: u64,
            pti_policy: i32,
            pti_faults: i32,
            pti_pageins: i32,
            pti_cow_faults: i32,
            pti_messages_sent: i32,
            pti_messages_received: i32,
            pti_syscalls_mach: i32,
            pti_syscalls_unix: i32,
            pti_csw: i32,
            pti_threadnum: i32,
            pti_numrunning: i32,
            pti_priority: i32,
        }

        let mut ti: ProcTaskInfo = std::mem::zeroed();
        let size = libc::proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            &mut ti as *mut _ as *mut libc::c_void,
            std::mem::size_of::<ProcTaskInfo>() as i32,
        );

        if size <= 0 {
            return None;
        }

        let mut name_buf: [libc::c_char; MAXCOMLEN * 2] = [0; MAXCOMLEN * 2];
        let ret = libc::proc_name(pid, name_buf.as_mut_ptr() as *mut libc::c_void, (MAXCOMLEN * 2) as u32);
        let comm = if ret == 0 {
            std::ffi::CStr::from_ptr(name_buf.as_ptr())
                .to_string_lossy()
                .to_string()
        } else {
            "?".to_string()
        };

        // Use libproc to get ppid via proc_pidinfo with PROC_PIDT_SHORTBSDINFO
        const PROC_PIDT_SHORTBSDINFO: libc::c_int = 13;
        #[repr(C)]
        struct ProcBsdShortInfo {
            pbsi_pid: u32,
            pbsi_ppid: u32,
            pbsi_pgid: u32,
            pbsi_status: u32,
            pbsi_comm: [u8; MAXCOMLEN],
            pbsi_flags: u32,
            pbsi_uid: u32,
            pbsi_gid: u32,
            pbsi_ruid: u32,
            pbsi_rgid: u32,
            pbsi_svuid: u32,
            pbsi_svgid: u32,
            pbsi_rfu: u32,
        }

        let ppid = {
            let mut bsd: ProcBsdShortInfo = std::mem::zeroed();
            let sz = libc::proc_pidinfo(
                pid,
                PROC_PIDT_SHORTBSDINFO,
                0,
                &mut bsd as *mut _ as *mut libc::c_void,
                std::mem::size_of::<ProcBsdShortInfo>() as i32,
            );
            if sz > 0 {
                bsd.pbsi_ppid
            } else {
                0
            }
        };

        Some(ProcInfo {
            pid: pid as u32,
            ppid,
            comm,
            rss: ti.pti_resident_size / 1024,
            cpu_pct: (ti.pti_total_user + ti.pti_total_system) as f64 / 1_000_000_000.0
                * 100.0,
        })
    }
}

pub(crate) async fn ssh_exec_russh(
    host: &str,
    port: u16,
    user: &str,
    command: &str,
    key_path: Option<&str>,
) -> CommandOutput {
    use russh::client;
    use russh_keys::load_secret_key;
    use std::sync::Arc;

    struct SshHandler;

    #[async_trait::async_trait]
    impl client::Handler for SshHandler {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &russh_keys::key::PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    let config = client::Config::default();
    let config = Arc::new(config);
    let handler = SshHandler;

    let addr = format!("{}:{}", host, port);
    let mut handle = match client::connect(config, &addr, handler).await {
        Ok(h) => h,
        Err(e) => {
            return CommandOutput::error(format!("ssh: connect failed: {}\n", e), 1);
        }
    };

    if let Some(key_file) = key_path {
        let key = match load_secret_key(key_file, None) {
            Ok(k) => k,
            Err(e) => {
                return CommandOutput::error(format!("ssh: cannot load key: {}\n", e), 1);
            }
        };
        if let Err(e) = handle
            .authenticate_publickey(user, Arc::new(key))
            .await
        {
            return CommandOutput::error(format!("ssh: auth error: {}\n", e), 1);
        }
    } else {
        return CommandOutput::error(
            "ssh: password auth not implemented, use -i keyfile\n".to_string(),
            1,
        );
    }

    let mut channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            return CommandOutput::error(format!("ssh: session open failed: {}\n", e), 1);
        }
    };

    if let Err(e) = channel.exec(true, command).await {
        return CommandOutput::error(format!("ssh: exec failed: {}\n", e), 1);
    }

    let mut stdout = Vec::new();
    let mut stderr_data = Vec::new();
    let mut exit_code: i32 = -1;

    loop {
        let msg = match channel.wait().await {
            Some(m) => m,
            None => break,
        };

        match msg {
            russh::ChannelMsg::Data { data } => {
                stdout.extend_from_slice(&data);
            }
            russh::ChannelMsg::ExtendedData { data, .. } => {
                stderr_data.extend_from_slice(&data);
            }
            russh::ChannelMsg::ExitStatus { exit_status } => {
                exit_code = exit_status as i32;
            }
            russh::ChannelMsg::Eof => {
                break;
            }
            _ => {}
        }
    }

    CommandOutput {
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr_data).to_string(),
        exit_code,
    }
}

pub(crate) fn parse_signal(name: &str) -> Option<i32> {
    let upper = name.to_uppercase();
    if !upper.starts_with("SIG") {
        return parse_signal(&format!("SIG{}", name));
    }
    match upper.as_str() {
        "SIGHUP" => Some(1),
        "SIGINT" => Some(2),
        "SIGQUIT" => Some(3),
        "SIGILL" => Some(4),
        "SIGTRAP" => Some(5),
        "SIGABRT" => Some(6),
        "SIGBUS" => Some(7),
        "SIGFPE" => Some(8),
        "SIGKILL" => Some(9),
        "SIGUSR1" => Some(10),
        "SIGSEGV" => Some(11),
        "SIGUSR2" => Some(12),
        "SIGPIPE" => Some(13),
        "SIGALRM" => Some(14),
        "SIGTERM" => Some(15),
        "SIGCHLD" => Some(17),
        "SIGCONT" => Some(18),
        "SIGSTOP" => Some(19),
        "SIGTSTP" => Some(20),
        "SIGTTIN" => Some(21),
        "SIGTTOU" => Some(22),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_shell_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_pwd() {
        let shell = mk_shell();
        let out = shell.cmd_pwd(&[]);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "/");
    }

    #[test]
    fn test_mkdir_cd() {
        let mut shell = mk_shell();
        shell.cmd_mkdir(&["testdir"]);
        let out = shell.cmd_cd(&["testdir"]);
        assert_eq!(out.exit_code, 0);
        assert_eq!(shell.cwd, "/testdir");
        let out = shell.cmd_pwd(&[]);
        assert_eq!(out.stdout.trim(), "/testdir");
    }

    #[test]
    fn test_ls_empty() {
        let shell = mk_shell();
        let out = shell.cmd_ls(&[]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_create_and_ls() {
        let shell = mk_shell();

        let r = shell.cmd_mkdir(&["subdir"]);
        assert_eq!(r.exit_code, 0, "mkdir failed: {:?}", r);

        let r = shell.cmd_touch(&["file1.txt"]);
        assert_eq!(r.exit_code, 0, "touch file1 failed: {:?}", r);

        let r = shell.cmd_touch(&["file2.txt"]);
        assert_eq!(r.exit_code, 0, "touch file2 failed: {:?}", r);

        let out = shell.cmd_ls(&[]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("subdir"));
        assert!(out.stdout.contains("file1.txt"));
        assert!(out.stdout.contains("file2.txt"));

        let out = shell.cmd_ls(&["-l"]);
        assert!(out.stdout.contains("file1.txt"));
        assert!(out.stdout.contains("subdir"));

        let out = shell.cmd_ls(&["-a"]);
        assert!(out.stdout.contains("."));
    }

    #[test]
    fn test_cat() {
        let shell = mk_shell();
        let _ = shell.vfs.write("/test.txt", "", "hello world\nfoo bar\n");
        let out = shell.cmd_cat(&["test.txt"], None);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "hello world\nfoo bar\n");
    }

    #[test]
    fn test_rm() {
        let shell = mk_shell();
        shell.cmd_touch(&["deleteme.txt"]);
        assert!(shell.vfs.exists("deleteme.txt", &shell.cwd));
        let out = shell.cmd_rm(&["deleteme.txt"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.vfs.exists("deleteme.txt", &shell.cwd));
    }

    #[test]
    fn test_cp() {
        let shell = mk_shell();
        let _ = shell.vfs.write("/original.txt", "", "copy content");
        let out = shell.cmd_cp(&["original.txt", "copy.txt"]);
        assert_eq!(out.exit_code, 0);
        let content = shell.vfs.read_to_string("copy.txt", &shell.cwd).unwrap();
        assert_eq!(content, "copy content");
    }

    #[test]
    fn test_mv() {
        let shell = mk_shell();
        let _ = shell.vfs.write("/old.txt", "", "move me");
        let out = shell.cmd_mv(&["old.txt", "new.txt"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.vfs.exists("old.txt", &shell.cwd));
        assert_eq!(
            shell.vfs.read_to_string("new.txt", &shell.cwd).unwrap(),
            "move me"
        );
    }

    #[test]
    fn test_grep() {
        let shell = mk_shell();
        let _ = shell.vfs.write("/data.txt", "", "hello world\nfoo bar\nHELLO again\n");
        let out = shell.cmd_grep(&["hello", "data.txt"], None);
        assert_eq!(out.stdout, "hello world\n");

        let out = shell.cmd_grep(&["-i", "hello", "data.txt"], None);
        assert!(out.stdout.contains("hello world"));
        assert!(out.stdout.contains("HELLO again"));
    }

    #[test]
    fn test_find() {
        let shell = mk_shell();
        let r = shell.cmd_mkdir(&["a"]);
        assert_eq!(r.exit_code, 0, "mkdir a failed: {:?}", r);
        let r = shell.cmd_mkdir(&["a/b"]);
        assert_eq!(r.exit_code, 0, "mkdir a/b failed: {:?}", r);
        let r = shell.cmd_touch(&["a/file.txt"]);
        assert_eq!(r.exit_code, 0, "touch a/file.txt failed: {:?}", r);
        let r = shell.cmd_touch(&["a/b/nested.txt"]);
        assert_eq!(r.exit_code, 0, "touch a/b/nested.txt failed: {:?}", r);

        let out = shell.cmd_find(&["a"]);
        assert!(out.stdout.contains("a/file.txt"));
        assert!(out.stdout.contains("a/b/nested.txt"));

        let out = shell.cmd_find(&["a", "-name", "*.txt", "-type", "f"]);
        assert!(out.stdout.contains("a/file.txt"));
    }

    #[test]
    fn test_echo() {
        let shell = mk_shell();
        let out = shell.cmd_echo(&["hello", "world"]);
        assert_eq!(out.stdout, "hello world\n");
    }

    #[test]
    fn test_chmod() {
        let shell = mk_shell();
        let r = shell.cmd_touch(&["script.sh"]);
        assert_eq!(r.exit_code, 0, "touch failed: {:?}", r);

        let out = shell.cmd_chmod(&["755", "script.sh"]);
        assert_eq!(out.exit_code, 0, "chmod failed: {:?}", out);

        let target = shell.vfs.resolve("script.sh", &shell.cwd).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = target.metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
    }

    #[test]
    fn test_kill_signal_parsing() {
        assert_eq!(parse_signal("SIGTERM"), Some(15));
        assert_eq!(parse_signal("TERM"), Some(15));
        assert_eq!(parse_signal("9"), None);
        assert_eq!(parse_signal("SIGKILL"), Some(9));
        assert_eq!(parse_signal("KILL"), Some(9));
        assert_eq!(parse_signal("HUP"), Some(1));
    }

    #[test]
    fn test_kill_usage_error() {
        let shell = mk_shell();
        let out = shell.cmd_kill(&[]);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("usage"));
    }

    #[test]
    fn test_kill_invalid_signal() {
        let shell = mk_shell();
        let out = shell.cmd_kill(&["-INVALID", "1"]);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_curl_http_get() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &["http://httpbin.org/get?test=1"], None);
        // httpbin might not always be available, but curl command should be recognized
        if out.exit_code == 0 {
            assert!(out.stdout.contains("test") || !out.stdout.is_empty());
        }
    }

    #[test]
    fn test_gzip_compress_decompress() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/data.txt", "", "hello gzip world! " .repeat(100).as_str())
            .unwrap();

        let out = shell.cmd_gzip(&["/data.txt"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.vfs.exists("/data.txt.gz", ""));
        assert!(!shell.vfs.exists("/data.txt", ""));

        let out = shell.cmd_gunzip(&["/data.txt.gz"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.vfs.exists("/data.txt", ""));
        let content = shell.vfs.read_to_string("/data.txt", "").unwrap();
        assert!(content.contains("hello gzip world!"));
    }

    #[test]
    fn test_gzip_stdout() {
        let shell = mk_shell();
        shell.vfs.write("/small.txt", "", "hi").unwrap();
        let out = shell.cmd_gzip(&["-c", "/small.txt"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.vfs.exists("/small.txt", ""));
    }

    #[test]
    fn test_tar_create_extract_list() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "alpha").unwrap();
        shell.vfs.write("/b.txt", "", "beta").unwrap();
        shell.vfs.create_dir("/sub", "").unwrap();
        shell.vfs.write("/sub/c.txt", "", "gamma").unwrap();

        let out = shell.cmd_tar(&["-cf", "test.tar", "a.txt", "b.txt", "sub"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.vfs.exists("test.tar", ""));

        let out = shell.cmd_tar(&["-tf", "test.tar"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("a.txt"));
        assert!(out.stdout.contains("b.txt"));
        assert!(out.stdout.contains("sub"));

        shell.vfs.remove_file("a.txt", "").unwrap();
        shell.vfs.remove_file("b.txt", "").unwrap();
        shell.vfs.remove_dir_all("sub", "").unwrap();

        let out = shell.cmd_tar(&["-xf", "test.tar"]);
        assert_eq!(out.exit_code, 0);
        assert_eq!(
            shell.vfs.read_to_string("a.txt", "").unwrap(),
            "alpha"
        );
        assert_eq!(
            shell.vfs.read_to_string("b.txt", "").unwrap(),
            "beta"
        );
        assert_eq!(
            shell.vfs.read_to_string("sub/c.txt", "").unwrap(),
            "gamma"
        );
    }

    #[test]
    fn test_tar_gzip() {
        let shell = mk_shell();
        shell.vfs.write("/hello.txt", "", "world").unwrap();

        let out = shell.cmd_tar(&["-czf", "test.tar.gz", "hello.txt"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.vfs.exists("test.tar.gz", ""));

        shell.vfs.remove_file("hello.txt", "").unwrap();

        let out = shell.cmd_tar(&["-xzf", "test.tar.gz"]);
        assert_eq!(out.exit_code, 0);
        assert_eq!(
            shell.vfs.read_to_string("hello.txt", "").unwrap(),
            "world"
        );
    }

    #[test]
    fn test_ping_localhost() {
        let mut shell = mk_shell();
        let out = shell.execute("ping", &["-c", "1", "-W", "1", "127.0.0.1"], None);
        if out.exit_code == 0 {
            assert!(out.stdout.contains("packets transmitted"));
        }
    }

    #[test]
    fn test_ping_missing_host() {
        let mut shell = mk_shell();
        let out = shell.execute("ping", &[], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
#[cfg(feature = "git")]
    fn test_git_init_and_status() {
        let mut shell = mk_shell();
        let out = shell.execute("git", &["init"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Initialized"));

        let out = shell.execute("git", &["status"], None);
        assert_eq!(out.exit_code, 0);

        shell.cmd_touch(&["test.txt"]);
        let out = shell.execute("git", &["add", "."], None);
        assert_eq!(out.exit_code, 0);

        let out = shell.execute("git", &["status"], None);
        assert!(out.stdout.contains("test.txt"));
    }

    #[test]
#[cfg(feature = "git")]
    fn test_git_clone_usage_error() {
        let mut shell = mk_shell();
        let out = shell.execute("git", &["clone"], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_ssh_missing_host() {
        let shell = mk_shell();
        let out = shell.cmd_ssh(&[]);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_head_tail() {
        let shell = mk_shell();
        let content = (1..=20).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\n");
        shell.vfs.write("/nums.txt", "", &content).unwrap();

        let out = shell.cmd_head(&["-n", "3", "/nums.txt"], None);
        assert_eq!(out.stdout.lines().count(), 3);
        assert!(out.stdout.contains("line1"));

        let out = shell.cmd_tail(&["-n", "3", "/nums.txt"], None);
        assert_eq!(out.stdout.lines().count(), 3);
        assert!(out.stdout.contains("line18"));
        assert!(out.stdout.contains("line20"));
    }

    #[test]
    fn test_wc() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "hello world\nfoo bar\nbaz").unwrap();
        let out = shell.cmd_wc(&["/a.txt"], None);
        assert!(out.stdout.contains("3"));
        assert!(out.stdout.contains("5"));
    }

    #[test]
    fn test_diff() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "line1\nline2\nline3\n").unwrap();
        shell.vfs.write("/b.txt", "", "line1\nmodified\nline3\n").unwrap();
        let out = shell.cmd_diff(&["/a.txt", "/b.txt"]);
        assert!(out.stdout.contains("modified"));
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_identical() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "same\n").unwrap();
        shell.vfs.write("/b.txt", "", "same\n").unwrap();
        let out = shell.cmd_diff(&["/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_sed() {
        let shell = mk_shell();
        shell.vfs.write("/t.txt", "", "hello world\nfoo bar\nhello again\n").unwrap();
        let out = shell.cmd_sed(&["s/hello/hi/g", "/t.txt"], None);
        assert!(out.stdout.contains("hi world"));
        assert!(out.stdout.contains("hi again"));
    }

    #[test]
    fn test_sort() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "c\na\nb\n").unwrap();
        let out = shell.cmd_sort(&["/f.txt"], None);
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_uniq() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "a\na\nb\nb\nb\nc\n").unwrap();
        let out = shell.cmd_uniq(&["/f.txt"], None);
        assert_eq!(out.stdout.trim().lines().count(), 3);
        let out = shell.cmd_uniq(&["-c", "/f.txt"], None);
        assert!(out.stdout.contains("2 a"));
        assert!(out.stdout.contains("3 b"));
    }

    #[test]
    fn test_which() {
        let shell = mk_shell();
        let out = shell.cmd_which(&["ls"]);
        assert!(out.stdout.contains("ls"));
    }

    #[test]
    fn test_curl_no_url_error() {
        let mut shell = mk_shell();
        let out = shell.execute("curl", &[], None);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("no URL"));
    }

    #[test]
    fn test_wget_no_url_error() {
        let mut shell = mk_shell();
        let out = shell.execute("wget", &[], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_extract_filename() {
        assert_eq!(extract_filename_from_url("http://example.com/file.txt"), "file.txt");
        assert_eq!(extract_filename_from_url("http://example.com/path/to/file.tar.gz"), "file.tar.gz");
        assert_eq!(extract_filename_from_url("http://example.com/"), "index.html");
        assert_eq!(extract_filename_from_url("http://example.com/a?q=1"), "a");
    }

    #[test]
    fn test_ps() {
        let mut shell = mk_shell();
        let out = shell.execute("ps", &[], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("PID"));
        assert!(out.stdout.contains("COMMAND"));
    }

    #[test]
    fn test_unknown_command_subprocess() {
        let mut shell = mk_shell();
        let out = shell.execute("echo", &["hello_from_subprocess"], None);
        assert_eq!(out.stdout.trim(), "hello_from_subprocess");
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_command_not_found() {
        let mut shell = mk_shell();
        let out = shell.execute("nonexistent_command_xyz", &[], None);
        assert_ne!(out.exit_code, 0);
    }
}
