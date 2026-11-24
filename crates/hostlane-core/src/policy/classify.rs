//! Risk assessment of command lines and remote paths (S1). The analysis is static and
//! errs on the side of a higher level: anything it does not recognise is at least L1,
//! and a command line it cannot parse is L2.

use hostlane_protocol::Risk;

use super::shell::{self, SimpleCommand};

const MAX_DEPTH: usize = 6;

/// Directories whose contents belong to the system rather than to a user or an app.
const SYSTEM_DIRS: &[&str] = &[
    "/etc",
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/boot",
    "/sys",
    "/proc",
    "/root",
    "/var/lib",
    "/var/spool",
    "/var/log",
    "/var/cache",
];

const READ_ONLY: &[&str] = &[
    "ls",
    "ll",
    "la",
    "dir",
    "cat",
    "tac",
    "head",
    "tail",
    "less",
    "more",
    "grep",
    "egrep",
    "fgrep",
    "zgrep",
    "rg",
    "ag",
    "ack",
    "stat",
    "file",
    "wc",
    "du",
    "df",
    "free",
    "uptime",
    "uname",
    "whoami",
    "id",
    "groups",
    "ps",
    "pgrep",
    "pidof",
    "pstree",
    "top",
    "htop",
    "atop",
    "lsof",
    "ss",
    "netstat",
    "ping",
    "ping6",
    "traceroute",
    "tracepath",
    "mtr",
    "dig",
    "nslookup",
    "host",
    "whois",
    "echo",
    "printf",
    "true",
    "false",
    "test",
    "[",
    "[[",
    "date",
    "printenv",
    "which",
    "whereis",
    "type",
    "pwd",
    "pushd",
    "popd",
    "dirs",
    "uniq",
    "cut",
    "awk",
    "gawk",
    "mawk",
    "jq",
    "yq",
    "tr",
    "column",
    "xxd",
    "od",
    "hexdump",
    "strings",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "b2sum",
    "cksum",
    "base64",
    "diff",
    "cmp",
    "comm",
    "tree",
    "realpath",
    "readlink",
    "basename",
    "dirname",
    "nproc",
    "lscpu",
    "lsblk",
    "blkid",
    "lsusb",
    "lspci",
    "lsmod",
    "lsns",
    "last",
    "lastlog",
    "w",
    "who",
    "users",
    "getent",
    "sleep",
    "history",
    "locale",
    "tty",
    "seq",
    "expr",
    "bc",
    "cal",
    "nl",
    "fold",
    "fmt",
    "paste",
    "join",
    "split",
    "zcat",
    "bzcat",
    "xzcat",
    "zless",
    "vmstat",
    "iostat",
    "mpstat",
    "sar",
    "pidstat",
    "nc",
    "ncat",
    "netcat",
    "telnet",
    "openssl",
    "ssh-keygen",
    "ssh-keyscan",
    "nmap",
    "mysqldump",
    "pg_dump",
    "pg_dumpall",
    "pg_isready",
    "getconf",
    "arch",
    "env",
    "export",
    "set",
    "unset",
    "alias",
    "read",
    "wait",
    "jobs",
    "hash",
    "shift",
    "local",
    "declare",
    "trap",
    "ulimit",
    "umask",
    "exit",
    "return",
    "break",
    "continue",
    "systemd-analyze",
    "loginctl",
    "busctl",
    "free",
    "dmidecode",
    "lshw",
    "sensors",
    "ethtool",
    "iwconfig",
    "arp",
    "curlie",
    "http",
    "xmllint",
    "tput",
    "clear",
    "column",
    "numfmt",
    "time",
    "sync",
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Assessment {
    pub risk: Risk,
    notes: Vec<(Risk, String)>,
    /// Remote paths the call names or writes, for `path` rules.
    pub paths: Vec<String>,
}

impl Assessment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raise(&mut self, risk: Risk, reason: impl Into<String>) {
        self.risk = self.risk.max(risk);
        let reason = reason.into();
        if risk > Risk::L0 && !self.notes.iter().any(|(_, r)| *r == reason) {
            self.notes.push((risk, reason));
        }
    }

    pub fn add_path(&mut self, path: &str) {
        if !path.is_empty() && !self.paths.iter().any(|p| p == path) {
            self.paths.push(path.to_string());
        }
    }

    /// Why the call has its risk, highest level first.
    pub fn reasons(&self) -> Vec<String> {
        let mut notes = self.notes.clone();
        notes.sort_by_key(|note| std::cmp::Reverse(note.0));
        notes.into_iter().map(|(_, reason)| reason).collect()
    }
}

/// Assesses a command line run by the remote shell. `cwd` resolves relative paths;
/// `stdin` is analysed when the command is a bare shell reading it.
pub fn command(line: &str, cwd: Option<&str>, stdin: Option<&str>) -> Assessment {
    let mut analyzer = Analyzer {
        assessment: Assessment::new(),
        cwd: cwd.filter(|c| c.starts_with('/')).map(normalize),
        stdin: stdin.map(str::to_owned),
    };
    analyzer.line(line, 0);
    analyzer.assessment
}

/// Assesses writing, replacing or deleting the file at `path`.
pub fn file_write(path: &str) -> Assessment {
    let mut assessment = Assessment::new();
    let path = normalize(path);
    assessment.add_path(&path);
    let (risk, reason) = path_risk(&path);
    assessment.raise(risk.max(Risk::L1), reason);
    assessment
}

/// Assesses reading the file or directory at `path`.
pub fn file_read(path: &str) -> Assessment {
    let mut assessment = Assessment::new();
    let path = normalize(path);
    assessment.add_path(&path);
    if is_secret(&path) {
        assessment.raise(Risk::L1, format!("may read secrets from {path}"));
    }
    assessment
}

#[derive(Debug, Clone, Copy, Default)]
struct Pipe {
    /// Standard input comes from the previous command.
    piped: bool,
    /// An earlier command of the pipeline downloads something.
    download: bool,
}

struct Analyzer {
    assessment: Assessment,
    cwd: Option<String>,
    stdin: Option<String>,
}

impl Analyzer {
    fn raise(&mut self, risk: Risk, reason: impl Into<String>) {
        self.assessment.raise(risk, reason);
    }

    fn line(&mut self, input: &str, depth: usize) {
        if depth > MAX_DEPTH {
            self.raise(Risk::L2, "commands are nested too deeply to analyse");
            return;
        }
        let parsed = shell::parse(input);
        if let Some(error) = &parsed.error {
            self.raise(
                Risk::L2,
                format!("cannot analyse the command line ({error})"),
            );
        }
        let mut download = false;
        for command in &parsed.commands {
            if !command.piped {
                download = false;
            }
            let pipe = Pipe {
                piped: command.piped,
                download,
            };
            let name = self.simple(command, depth, pipe);
            if matches!(name.as_str(), "curl" | "wget" | "fetch") {
                download = true;
            }
        }
        for body in &parsed.substitutions {
            self.line(body, depth + 1);
        }
    }

    fn simple(&mut self, command: &SimpleCommand, depth: usize, pipe: Pipe) -> String {
        for target in &command.writes {
            self.write_target(target);
        }
        let words: Vec<&str> = command.words.iter().map(String::as_str).collect();
        self.words(&words, depth, pipe)
    }

    /// Analyses one command given as words; returns the program name.
    fn words(&mut self, words: &[&str], depth: usize, pipe: Pipe) -> String {
        if depth > MAX_DEPTH {
            self.raise(Risk::L2, "commands are nested too deeply to analyse");
            return String::new();
        }
        let start = words.iter().take_while(|w| is_assignment(w)).count();
        let words = &words[start..];
        let Some(first) = words.first() else {
            return String::new();
        };
        let name = program_name(first).to_string();
        let args = &words[1..];
        for arg in operands(args, &[]) {
            if is_secret(arg) {
                let path = self.resolve(arg);
                self.raise(Risk::L1, format!("may read secrets from {path}"));
            }
            if looks_like_path(arg) {
                let path = self.resolve(arg);
                self.assessment.add_path(&path);
            }
        }
        if args.len() == 1 && matches!(args[0], "--version" | "--help" | "-V") {
            return name;
        }
        self.program(&name, args, depth, pipe);
        name
    }

    fn program(&mut self, name: &str, args: &[&str], depth: usize, pipe: Pipe) {
        let next = depth + 1;
        match name {
            "sudo" | "doas" | "sudoedit" => {
                self.raise(Risk::L2, format!("{name}: runs as root"));
                let rest = skip_options(
                    args,
                    &[
                        "-u",
                        "-g",
                        "-h",
                        "-p",
                        "-C",
                        "-D",
                        "-r",
                        "-t",
                        "-U",
                        "-T",
                        "--user",
                        "--group",
                        "--host",
                        "--prompt",
                        "--chdir",
                        "--role",
                        "--type",
                        "--other-user",
                        "--close-from",
                        "--command-timeout",
                    ],
                );
                if name == "sudoedit" || has_flag(args, 'e', &["--edit"]) {
                    for file in rest {
                        self.write_target(file);
                    }
                } else if rest.is_empty() {
                    if has_flag(args, 'i', &["--login"]) || has_flag(args, 's', &["--shell"]) {
                        self.raise(Risk::L2, "opens a root shell");
                    }
                } else {
                    self.words(rest, next, pipe);
                }
            }
            "su" | "runuser" => {
                self.raise(Risk::L2, format!("{name}: switches user"));
                if let Some(script) = option_values(args, Some('c'), &["--command"]).first() {
                    self.line(script, next);
                } else if let Some(index) = args.iter().position(|a| *a == "--") {
                    self.words(&args[index + 1..], next, pipe);
                }
            }
            "env" => {
                let rest = skip_options(
                    args,
                    &["-u", "--unset", "-C", "--chdir", "-S", "--split-string"],
                );
                if let Some(script) = option_values(args, Some('S'), &["--split-string"]).first() {
                    self.line(script, next);
                }
                let skip = rest.iter().take_while(|w| is_assignment(w)).count();
                self.words(&rest[skip..], next, pipe);
            }
            "nohup" | "exec" | "builtin" | "setsid" | "unbuffer" | "caffeinate" => {
                self.words(skip_options(args, &[]), next, pipe);
            }
            "time" => {
                self.words(
                    skip_options(args, &["-f", "--format", "-o", "--output"]),
                    next,
                    pipe,
                );
            }
            "command" => {
                if !has_flag(args, 'v', &[]) && !has_flag(args, 'V', &[]) {
                    self.words(skip_options(args, &[]), next, pipe);
                }
            }
            "nice" => {
                self.words(skip_options(args, &["-n", "--adjustment"]), next, pipe);
            }
            "ionice" => {
                self.words(
                    skip_options(
                        args,
                        &["-c", "-n", "-p", "-P", "-u", "--class", "--classdata"],
                    ),
                    next,
                    pipe,
                );
            }
            "stdbuf" => {
                self.words(skip_options(args, &["-i", "-o", "-e"]), next, pipe);
            }
            "timeout" => {
                let rest = skip_options(args, &["-s", "--signal", "-k", "--kill-after"]);
                self.words(rest.get(1..).unwrap_or_default(), next, pipe);
            }
            "flock" => {
                let rest = skip_options(args, &["-w", "--timeout", "-E", "--conflict-exit-code"]);
                let rest = rest.get(1..).unwrap_or_default();
                match rest {
                    ["-c" | "--command", script, ..] => self.line(script, next),
                    _ => {
                        self.words(rest, next, pipe);
                    }
                }
            }
            "chroot" => {
                self.raise(Risk::L2, "runs a command in a chroot");
                let rest = skip_options(args, &["--userspec", "--groups"]);
                self.words(rest.get(1..).unwrap_or_default(), next, pipe);
            }
            "xargs" => {
                let rest = skip_options(
                    args,
                    &[
                        "-I",
                        "-n",
                        "-P",
                        "-L",
                        "-l",
                        "-d",
                        "-a",
                        "-s",
                        "-E",
                        "--max-args",
                        "--max-procs",
                        "--delimiter",
                        "--arg-file",
                        "--max-chars",
                        "--replace",
                        "--eof",
                        "--max-lines",
                    ],
                );
                self.words(rest, next, Pipe::default());
            }
            "watch" => {
                let rest = skip_options(args, &["-n", "--interval", "-d", "--differences"]);
                if !rest.is_empty() {
                    self.line(&rest.join(" "), next);
                }
            }
            "eval" => self.line(&args.join(" "), next),
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "mksh" | "fish" | "busybox"
                if name != "busybox" || args.first() == Some(&"sh") =>
            {
                let args = if name == "busybox" { &args[1..] } else { args };
                self.shell(name, args, depth, pipe);
            }
            "busybox" => {
                self.words(args, next, pipe);
            }
            "python" | "python2" | "python3" | "perl" | "ruby" | "node" | "nodejs" | "php"
            | "deno" | "bun" | "lua" | "Rscript" | "osascript" => {
                self.interpreter(name, args, pipe);
            }
            "source" | "." => self.raise(Risk::L1, "runs a script file"),
            "cd" => self.change_dir(args.first().copied()),
            _ => self.tool(name, args, depth, pipe),
        }
    }

    fn shell(&mut self, name: &str, args: &[&str], depth: usize, pipe: Pipe) {
        let mut index = 0;
        while index < args.len() {
            let arg = args[index];
            if arg == "--" {
                index += 1;
                break;
            }
            if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                if arg.contains('c') {
                    match args.get(index + 1) {
                        Some(script) => self.line(script, depth + 1),
                        None => self.raise(Risk::L2, format!("{name} -c without a command")),
                    }
                    return;
                }
                if arg == "-o" {
                    index += 1;
                }
                index += 1;
                continue;
            }
            if arg.starts_with("--") || arg.starts_with('+') {
                index += 1;
                continue;
            }
            break;
        }
        if index < args.len() {
            self.raise(Risk::L1, format!("{name}: runs the script {}", args[index]));
        } else {
            self.stdin_input(name, pipe, depth);
        }
    }

    fn interpreter(&mut self, name: &str, args: &[&str], pipe: Pipe) {
        if name == "perl" && has_flag(args, 'i', &[]) {
            self.raise(Risk::L1, "perl -i edits files in place");
        }
        if has_flag(args, 'c', &[]) && name.starts_with("python")
            || has_flag(args, 'e', &["--eval"])
            || has_flag(args, 'E', &[])
            || has_flag(args, 'r', &[]) && name == "php"
        {
            self.raise(Risk::L1, format!("{name}: runs inline code"));
            return;
        }
        let rest = skip_options(args, &["-m", "-W", "-X", "-I", "-M"]);
        if has_flag(args, 'm', &[]) {
            self.raise(Risk::L1, format!("{name}: runs a module"));
        } else if let Some(script) = rest.first() {
            self.raise(Risk::L1, format!("{name}: runs the script {script}"));
        } else {
            self.stdin_input(name, pipe, 0);
        }
    }

    /// A shell or interpreter reading its program from standard input.
    fn stdin_input(&mut self, name: &str, pipe: Pipe, depth: usize) {
        if pipe.piped {
            if pipe.download {
                self.raise(Risk::L3, format!("pipes a download into {name}"));
            } else {
                self.raise(Risk::L2, format!("{name} runs commands read from a pipe"));
            }
        } else if let Some(stdin) = self.stdin.take() {
            self.line(&stdin, depth + 1);
        } else {
            self.raise(Risk::L1, format!("opens {name}"));
        }
    }

    fn change_dir(&mut self, target: Option<&str>) {
        self.cwd = match target {
            Some(dir) if dir.starts_with('/') => Some(normalize(dir)),
            Some(dir) if !dir.starts_with('~') && !dir.starts_with('-') && !dir.contains('$') => {
                self.cwd
                    .as_ref()
                    .map(|cwd| normalize(&format!("{cwd}/{dir}")))
            }
            _ => None,
        };
    }

    fn resolve(&self, path: &str) -> String {
        if path.starts_with('/') {
            normalize(path)
        } else if path.starts_with('~') || path.starts_with('$') {
            path.to_string()
        } else if let Some(cwd) = &self.cwd {
            normalize(&format!("{cwd}/{path}"))
        } else {
            path.to_string()
        }
    }

    fn write_target(&mut self, target: &str) {
        if target == "-" {
            return;
        }
        let path = self.resolve(target);
        if is_harmless_device(&path) {
            return;
        }
        self.assessment.add_path(&path);
        let (risk, reason) = path_risk(&path);
        self.raise(risk.max(Risk::L1), reason);
    }

    fn tool(&mut self, name: &str, args: &[&str], depth: usize, pipe: Pipe) {
        let ops = operands(args, &[]);
        let first = ops.first().copied();
        match name {
            _ if READ_ONLY.contains(&name) => {}
            "hostname" => {
                if !ops.is_empty() {
                    self.raise(Risk::L2, "changes the host name");
                }
            }
            "dmesg" => {
                if has_flag(args, 'c', &["--clear", "--read-clear"]) || has_flag(args, 'C', &[]) {
                    self.raise(Risk::L2, "clears the kernel log");
                }
            }
            "sort" => {
                for file in option_values(args, Some('o'), &["--output"]) {
                    self.write_target(&file);
                }
            }
            "sed" => {
                if has_flag(args, 'i', &["--in-place"]) {
                    let ops = operands(args, &["-e", "--expression", "-f", "--file", "-l"]);
                    let explicit =
                        has_flag(args, 'e', &["--expression"]) || has_flag(args, 'f', &["--file"]);
                    let skip = if explicit { 0 } else { 1 };
                    for file in ops.iter().skip(skip) {
                        self.write_target(file);
                    }
                    self.raise(Risk::L1, "sed -i edits files in place");
                }
            }
            "tee" => {
                for file in ops {
                    self.write_target(file);
                }
            }
            "cp" | "mv" | "install" | "ln" | "rsync" | "scp" | "rename" => {
                self.copy(name, args);
            }
            "touch" | "mkdir" | "truncate" | "mkfifo" | "mknod" | "setfacl" | "chattr" => {
                let ops = operands(args, &["-m", "--mode", "-s", "--size", "-d", "-t", "-r"]);
                for file in ops {
                    if name == "chattr" && (file.starts_with('+') || file.starts_with('=')) {
                        continue;
                    }
                    self.write_target(file);
                }
                if name == "chattr" {
                    self.raise(Risk::L2, "changes file attributes");
                }
            }
            "rm" | "unlink" | "rmdir" | "shred" | "srm" => self.remove(name, args),
            "chmod" | "chown" | "chgrp" => self.permissions(name, args),
            "find" => self.find(args, depth),
            "dd" => self.dd(args),
            "curl" => self.curl(args),
            "wget" => self.wget(args),
            "tar" | "bsdtar" | "gtar" => self.tar(args),
            "unzip" => {
                if !has_flag(args, 'l', &[])
                    && !has_flag(args, 't', &[])
                    && !has_flag(args, 'Z', &[])
                {
                    self.extract_into(option_values(args, Some('d'), &[]).first());
                }
            }
            "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz" | "zstd" | "unzstd" | "zip"
            | "7z" | "unrar" => {
                let reads = ["-l", "-t", "-c", "--list", "--test", "--stdout"]
                    .iter()
                    .any(|flag| args.contains(flag))
                    || has_flag(args, 'c', &[]) && name != "7z";
                if !reads {
                    for file in ops {
                        self.write_target(file);
                    }
                }
            }
            "kill" => self.kill(args),
            "pkill" | "killall" | "skill" | "fuser" => {
                if name != "fuser" || has_flag(args, 'k', &["--kill"]) {
                    self.raise(Risk::L1, format!("{name}: stops processes"));
                }
            }
            "shutdown" | "reboot" | "halt" | "poweroff" => {
                if name == "shutdown" && has_flag(args, 'c', &[]) {
                    self.raise(Risk::L2, "cancels a scheduled shutdown");
                } else {
                    self.raise(Risk::L3, format!("{name}: stops or reboots the server"));
                }
            }
            "init" | "telinit" => {
                if matches!(first, Some("0" | "6")) {
                    self.raise(Risk::L3, format!("{name}: stops or reboots the server"));
                } else {
                    self.raise(Risk::L2, format!("{name}: changes the runlevel"));
                }
            }
            _ if name.starts_with("mkfs") || is_disk_tool(name) => {
                let listing = has_flag(args, 'l', &["--list"]) || first == Some("print");
                if !listing {
                    self.raise(Risk::L3, format!("{name}: can erase a disk"));
                }
            }
            "systemctl" => self.systemctl(args),
            "service" => {
                let reads = args.contains(&"--status-all")
                    || matches!(ops.get(1), Some(&"status"))
                    || ops.is_empty();
                if !reads {
                    self.raise(
                        Risk::L2,
                        format!("service {}: changes system services", ops.join(" ")),
                    );
                }
            }
            "journalctl" => {
                if args
                    .iter()
                    .any(|a| a.starts_with("--vacuum") || *a == "--rotate" || *a == "--flush")
                {
                    self.raise(Risk::L2, "journalctl: deletes or rotates system logs");
                }
            }
            "docker" | "podman" | "nerdctl" => self.docker(name, args, depth),
            "docker-compose" | "podman-compose" => self.compose(name, args, depth),
            "git" => self.git(args),
            "apt" | "apt-get" | "aptitude" | "dpkg" | "yum" | "dnf" | "microdnf" | "rpm"
            | "apk" | "pacman" | "zypper" | "snap" | "flatpak" | "emerge" | "port" | "pkg" => {
                self.packages(name, args);
            }
            "pip" | "pip3" | "pipx" | "npm" | "npx" | "pnpm" | "yarn" | "cargo" | "gem" | "go"
            | "composer" | "bundle" | "make" | "cmake" | "ninja" | "mvn" | "gradle" | "uv"
            | "poetry" | "dotnet" => {
                const READS: &[&str] = &[
                    "list", "ls", "show", "freeze", "info", "view", "outdated", "search", "help",
                    "version", "check", "audit", "why", "config", "env", "tree", "doctor",
                ];
                if !first.is_some_and(|verb| READS.contains(&verb)) {
                    self.raise(Risk::L1, format!("{name}: builds, installs or runs code"));
                }
            }
            "useradd" | "userdel" | "usermod" | "adduser" | "deluser" | "groupadd" | "groupdel"
            | "groupmod" | "addgroup" | "delgroup" | "passwd" | "chpasswd" | "chsh" | "chfn"
            | "gpasswd" | "visudo" | "chage" => {
                self.raise(Risk::L2, format!("{name}: changes users or groups"));
            }
            "crontab" => {
                if has_flag(args, 'r', &[]) {
                    self.raise(Risk::L2, "crontab -r removes every scheduled job");
                } else if !has_flag(args, 'l', &[]) {
                    self.raise(Risk::L1, "crontab: changes scheduled jobs");
                }
            }
            "at" | "batch" => self.raise(Risk::L1, format!("{name}: schedules a command")),
            "ufw" => {
                if !matches!(first, None | Some("status" | "show" | "app")) {
                    self.raise(Risk::L2, "ufw: changes the firewall");
                }
            }
            "iptables" | "ip6tables" | "iptables-legacy" | "iptables-nft" | "ebtables" => {
                let reads =
                    has_flag(args, 'L', &["--list"]) || has_flag(args, 'S', &["--list-rules"]);
                if !reads {
                    self.raise(Risk::L2, format!("{name}: changes the firewall"));
                }
            }
            "iptables-save" | "ip6tables-save" => {}
            "iptables-restore" | "ip6tables-restore" => {
                self.raise(Risk::L2, format!("{name}: replaces the firewall rules"));
            }
            "nft" => {
                if !matches!(first, Some("list") | None) {
                    self.raise(Risk::L2, "nft: changes the firewall");
                }
            }
            "firewall-cmd" => {
                let reads = args.iter().all(|a| {
                    ["--list", "--get", "--state", "--query", "--info", "--zone"]
                        .iter()
                        .any(|p| a.starts_with(p))
                });
                if !reads {
                    self.raise(Risk::L2, "firewall-cmd: changes the firewall");
                }
            }
            "ip" => {
                const CHANGES: &[&str] = &[
                    "add", "del", "delete", "change", "replace", "set", "flush", "append",
                    "prepend", "up", "down", "exec",
                ];
                if ops.iter().any(|op| CHANGES.contains(op)) {
                    self.raise(Risk::L2, "ip: changes the network configuration");
                }
            }
            "ifconfig" | "route" | "ifup" | "ifdown" | "nmcli" | "netplan" | "brctl" | "tc" => {
                let reads = match name {
                    "ifconfig" => ops.len() <= 1,
                    "route" => !ops.iter().any(|op| matches!(*op, "add" | "del")),
                    "nmcli" => ops.iter().all(|op| {
                        matches!(
                            *op,
                            "show" | "status" | "list" | "device" | "connection" | "general"
                        )
                    }),
                    "netplan" => matches!(first, Some("get" | "info")),
                    "tc" => ops.contains(&"show"),
                    _ => false,
                };
                if !reads {
                    self.raise(
                        Risk::L2,
                        format!("{name}: changes the network configuration"),
                    );
                }
            }
            "mount" | "umount" | "swapon" | "swapoff" | "losetup" => {
                let reads = name == "mount" && ops.is_empty()
                    || name == "swapon"
                        && (has_flag(args, 's', &["--show", "--summary"]) || ops.is_empty())
                    || name == "losetup" && (ops.is_empty() || has_flag(args, 'l', &["--list"]));
                if !reads {
                    self.raise(Risk::L2, format!("{name}: changes mounted filesystems"));
                }
            }
            "sysctl" => {
                let writes = has_flag(args, 'w', &["--write"])
                    || has_flag(args, 'p', &["--load", "--system"])
                    || ops.iter().any(|op| op.contains('='));
                if writes {
                    self.raise(Risk::L2, "sysctl: changes kernel parameters");
                }
            }
            "modprobe"
            | "insmod"
            | "rmmod"
            | "depmod"
            | "update-grub"
            | "grub-install"
            | "grub-mkconfig"
            | "grub2-mkconfig"
            | "update-alternatives"
            | "update-rc.d"
            | "chkconfig"
            | "ldconfig"
            | "setenforce"
            | "setsebool"
            | "a2enmod"
            | "a2dismod"
            | "a2ensite"
            | "a2dissite"
            | "a2enconf"
            | "a2disconf"
            | "update-initramfs"
            | "dracut"
            | "locale-gen"
            | "systemd-run"
            | "update-ca-certificates"
            | "sestatus"
                if name != "sestatus" =>
            {
                self.raise(
                    Risk::L2,
                    format!("{name}: changes the system configuration"),
                );
            }
            "timedatectl" | "hostnamectl" | "localectl" => {
                if !matches!(
                    first,
                    None | Some(
                        "status"
                            | "show"
                            | "list-timezones"
                            | "list-locales"
                            | "list-keymaps"
                            | "timesync-status"
                            | "show-timesync"
                    )
                ) {
                    self.raise(
                        Risk::L2,
                        format!("{name}: changes the system configuration"),
                    );
                }
            }
            "certbot" | "letsencrypt" => {
                if !matches!(first, Some("certificates" | "plugins")) {
                    self.raise(Risk::L2, format!("{name}: changes TLS certificates"));
                }
            }
            "nginx" | "openresty" => {
                let reads = has_flag(args, 't', &[])
                    || has_flag(args, 'T', &[])
                    || has_flag(args, 'v', &[]);
                if has_flag(args, 's', &[]) || !reads {
                    self.raise(
                        Risk::L2,
                        format!("{name}: starts or signals the web server"),
                    );
                }
            }
            "apachectl" | "apache2ctl" | "httpd" | "apache2" => {
                let reads = ["-t", "-S", "-M", "-v", "-V", "configtest", "-D"]
                    .iter()
                    .any(|flag| args.contains(flag));
                if !reads {
                    self.raise(Risk::L2, format!("{name}: controls the web server"));
                }
            }
            "supervisorctl" => {
                if !matches!(
                    first,
                    None | Some("status" | "avail" | "tail" | "pid" | "version" | "help")
                ) {
                    self.raise(Risk::L2, "supervisorctl: controls services");
                }
            }
            "pm2" => {
                const READS: &[&str] = &[
                    "list",
                    "ls",
                    "status",
                    "logs",
                    "describe",
                    "show",
                    "info",
                    "monit",
                    "jlist",
                    "prettylist",
                    "l",
                ];
                if !first.is_some_and(|verb| READS.contains(&verb)) {
                    self.raise(Risk::L1, "pm2: manages processes");
                }
            }
            "mysql" | "mariadb" | "psql" | "sqlite3" | "clickhouse-client" | "clickhouse" => {
                self.database(name, args);
            }
            "redis-cli" | "valkey-cli" | "keydb-cli" => self.redis(args),
            "dropdb" | "dropuser" => {
                self.raise(Risk::L3, format!("{name}: drops a database or role"))
            }
            "createdb" | "createuser" | "pg_restore" | "mysqlimport" | "mysqladmin" | "pg_ctl"
            | "pg_ctlcluster" | "mongorestore" => {
                self.raise(Risk::L2, format!("{name}: changes a database"));
            }
            "vi" | "vim" | "nvim" | "nano" | "emacs" | "ed" | "pico" | "micro" | "joe" => {
                for file in ops {
                    self.write_target(file);
                }
                self.raise(Risk::L1, format!("{name}: opens an editor"));
            }
            "patch" => {
                for file in operands(args, &["-p", "-i", "--input", "-d", "--directory", "-o"]) {
                    self.write_target(file);
                }
                self.raise(Risk::L1, "patch: edits files");
            }
            "ssh" => {
                self.raise(Risk::L1, "ssh: runs commands on another host");
                let ops = operands(
                    args,
                    &[
                        "-p", "-i", "-l", "-o", "-F", "-J", "-L", "-R", "-D", "-W", "-b", "-c",
                        "-E", "-m", "-O", "-Q", "-S", "-w", "-e", "-B", "-I",
                    ],
                );
                if ops.len() > 1 {
                    self.line(&ops[1..].join(" "), depth + 1);
                }
            }
            _ => {
                let _ = pipe;
                self.raise(Risk::L1, format!("{name}: not a known read-only command"));
            }
        }
    }

    fn copy(&mut self, name: &str, args: &[&str]) {
        let with_values = [
            "-t",
            "--target-directory",
            "-S",
            "--suffix",
            "-m",
            "--mode",
            "-o",
            "-g",
            "-e",
            "--rsh",
            "-P",
            "--exclude",
            "--include",
            "-f",
            "--filter",
        ];
        let ops = operands(args, &with_values);
        if name == "rsync" && args.iter().any(|a| a.starts_with("--delete")) {
            self.raise(Risk::L2, "rsync --delete removes files at the destination");
        }
        let explicit = option_values(args, Some('t'), &["--target-directory"]);
        let mut writes: Vec<String> = explicit.clone();
        match name {
            "mv" | "rename" => writes.extend(ops.iter().map(|s| s.to_string())),
            "install" if has_flag(args, 'd', &["--directory"]) => {
                writes.extend(ops.iter().map(|s| s.to_string()))
            }
            "ln" if ops.len() == 1 => {}
            _ if explicit.is_empty() && ops.len() >= 2 => {
                writes.push(ops[ops.len() - 1].to_string());
            }
            _ => {}
        }
        for target in writes {
            if (name == "rsync" || name == "scp") && is_remote_spec(&target) {
                self.raise(Risk::L1, format!("{name}: copies to another host"));
                continue;
            }
            self.write_target(&target);
        }
        self.raise(Risk::L1, format!("{name}: changes files"));
    }

    fn remove(&mut self, name: &str, args: &[&str]) {
        let recursive =
            name == "rm" && (has_flag(args, 'r', &["--recursive"]) || has_flag(args, 'R', &[]));
        let targets = operands(args, &[]);
        if targets.is_empty() {
            if recursive {
                self.raise(Risk::L2, "rm -r on paths read from input");
            } else {
                self.raise(Risk::L1, format!("{name}: deletes files"));
            }
            return;
        }
        for target in targets {
            let path = self.resolve(target);
            self.assessment.add_path(&path);
            if is_block_device(&path) {
                self.raise(Risk::L3, format!("{name} on the device {path}"));
            } else if name == "shred" || name == "srm" {
                let risk = if is_system(&path) { Risk::L3 } else { Risk::L2 };
                self.raise(risk, format!("{name}: destroys the contents of {path}"));
            } else if recursive {
                if target == "{}" {
                    self.raise(Risk::L2, "rm -r on paths found by find");
                } else if is_broad(target, &path) {
                    self.raise(Risk::L3, format!("rm -r on {target}"));
                } else if is_system(&path) {
                    self.raise(Risk::L3, format!("rm -r in the system path {path}"));
                } else if path.starts_with('/') || path.starts_with('~') {
                    self.raise(Risk::L2, format!("deletes the directory tree {path}"));
                } else {
                    self.raise(Risk::L1, format!("deletes {path}"));
                }
            } else if is_system(&path) {
                self.raise(Risk::L2, format!("deletes {path}, a system path"));
            } else if target.contains('$') || target == "*" || target == "/*" {
                self.raise(
                    Risk::L2,
                    format!("deletes {target}, which the shell expands"),
                );
            } else {
                self.raise(Risk::L1, format!("deletes {path}"));
            }
        }
    }

    fn permissions(&mut self, name: &str, args: &[&str]) {
        let recursive = has_flag(args, 'R', &["--recursive"]);
        let ops = operands(args, &["--reference", "--from"]);
        let skip = if args.iter().any(|a| a.starts_with("--reference")) {
            0
        } else {
            1
        };
        for target in ops.iter().skip(skip) {
            let path = self.resolve(target);
            self.assessment.add_path(&path);
            if recursive && (is_broad(target, &path) || is_system(&path)) {
                self.raise(Risk::L3, format!("{name} -R on {path}"));
            } else if is_system(&path) || is_block_device(&path) {
                self.raise(Risk::L2, format!("{name} on the system path {path}"));
            } else {
                self.raise(Risk::L1, format!("{name} {path}"));
            }
        }
    }

    fn find(&mut self, args: &[&str], depth: usize) {
        let roots: Vec<&str> = args
            .iter()
            .take_while(|a| !a.starts_with('-') && **a != "(" && **a != "!")
            .copied()
            .collect();
        let mut index = 0;
        while index < args.len() {
            match args[index] {
                "-delete" => {
                    let roots = if roots.is_empty() {
                        vec!["."]
                    } else {
                        roots.clone()
                    };
                    for root in roots {
                        let path = self.resolve(root);
                        if is_broad(root, &path) && root != "." || is_system(&path) {
                            self.raise(Risk::L3, format!("find -delete under {path}"));
                        } else {
                            self.raise(Risk::L2, "find -delete removes the files it finds");
                        }
                    }
                }
                "-exec" | "-execdir" | "-ok" | "-okdir" => {
                    let end = args[index + 1..]
                        .iter()
                        .position(|a| *a == ";" || *a == "+")
                        .map_or(args.len(), |p| index + 1 + p);
                    let inner = args[index + 1..end].to_vec();
                    self.words(&inner, depth + 1, Pipe::default());
                    index = end;
                }
                "-fprint" | "-fprint0" | "-fprintf" | "-fls" => {
                    if let Some(file) = args.get(index + 1) {
                        self.write_target(file);
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn dd(&mut self, args: &[&str]) {
        let mut writes = false;
        for arg in args {
            if let Some(target) = arg.strip_prefix("of=") {
                writes = true;
                let path = self.resolve(target);
                if is_block_device(&path) {
                    self.assessment.add_path(&path);
                    self.raise(Risk::L3, format!("dd writes to the device {path}"));
                } else {
                    self.write_target(target);
                }
            }
        }
        if !writes {
            self.raise(Risk::L1, "dd: copies raw data");
        }
    }

    fn curl(&mut self, args: &[&str]) {
        for file in option_values(args, Some('o'), &["--output"]) {
            if file != "-" {
                self.write_target(&file);
            }
        }
        if has_flag(args, 'O', &["--remote-name", "--remote-name-all"]) {
            self.raise(Risk::L1, "curl: saves a download");
        }
        let method = option_values(args, Some('X'), &["--request"]);
        if let Some(method) = method.first()
            && !method.eq_ignore_ascii_case("GET")
            && !method.eq_ignore_ascii_case("HEAD")
        {
            self.raise(
                Risk::L1,
                format!("curl: sends a {} request", method.to_uppercase()),
            );
        }
        let sends = args.iter().any(|a| {
            a.starts_with("--data")
                || a.starts_with("--form")
                || *a == "--json"
                || *a == "--upload-file"
        }) || has_flag(args, 'd', &[])
            || has_flag(args, 'F', &[])
            || has_flag(args, 'T', &[]);
        if sends {
            self.raise(Risk::L1, "curl: sends data to a service");
        }
    }

    fn wget(&mut self, args: &[&str]) {
        if args.contains(&"--spider") {
            return;
        }
        let outputs = option_values(args, Some('O'), &["--output-document"]);
        if outputs.is_empty() {
            self.raise(Risk::L1, "wget: saves a download");
        }
        for file in outputs {
            if file != "-" {
                self.write_target(&file);
            }
        }
        if args.iter().any(|a| a.starts_with("--post")) || args.contains(&"--method") {
            self.raise(Risk::L1, "wget: sends data to a service");
        }
    }

    fn tar(&mut self, args: &[&str]) {
        let mut modes = String::new();
        if let Some(first) = args.first()
            && !first.starts_with('-')
        {
            modes.push_str(first);
        }
        for arg in args {
            if arg.starts_with('-') && !arg.starts_with("--") {
                modes.push_str(&arg[1..]);
            }
        }
        let long = |flag: &str| args.contains(&flag);
        if modes.contains('t') || long("--list") {
            return;
        }
        if modes.contains('x') || long("--extract") || long("--get") {
            let dir = option_values(args, Some('C'), &["--directory"]);
            self.extract_into(dir.first());
        } else {
            for file in option_values(args, Some('f'), &["--file"]) {
                if file != "-" {
                    self.write_target(&file);
                }
            }
            self.raise(Risk::L1, "tar: writes an archive");
        }
    }

    fn extract_into(&mut self, dir: Option<&String>) {
        match dir {
            Some(dir) => self.write_target(dir),
            None => match self.cwd.clone() {
                Some(cwd) => self.write_target(&cwd),
                None => self.raise(Risk::L1, "extracts files"),
            },
        }
    }

    fn kill(&mut self, args: &[&str]) {
        let mut targets = Vec::new();
        let mut index = 0;
        let mut signal_seen = false;
        while index < args.len() {
            let arg = args[index];
            if matches!(arg, "-s" | "-n" | "--signal") {
                index += 2;
                signal_seen = true;
                continue;
            }
            if arg.starts_with('-') && !signal_seen && arg != "-1" || arg == "-l" || arg == "-L" {
                signal_seen = true;
            } else if arg != "--" {
                targets.push(arg);
            }
            index += 1;
        }
        if targets.contains(&"-1") {
            self.raise(Risk::L3, "kill -1 signals every process");
        } else if targets.contains(&"1") {
            self.raise(Risk::L3, "kill signals init (PID 1)");
        } else if !targets.is_empty() {
            self.raise(Risk::L1, "kill: stops processes");
        }
    }

    fn systemctl(&mut self, args: &[&str]) {
        const READS: &[&str] = &[
            "status",
            "show",
            "cat",
            "list-units",
            "list-unit-files",
            "list-timers",
            "list-sockets",
            "list-dependencies",
            "list-jobs",
            "list-machines",
            "list-paths",
            "list-automounts",
            "is-active",
            "is-enabled",
            "is-failed",
            "is-system-running",
            "get-default",
            "show-environment",
            "help",
        ];
        const POWER: &[&str] = &[
            "reboot",
            "poweroff",
            "halt",
            "kexec",
            "emergency",
            "rescue",
            "isolate",
            "default",
            "soft-reboot",
            "suspend",
            "hibernate",
            "hybrid-sleep",
        ];
        let ops = operands(
            args,
            &[
                "-H",
                "--host",
                "-M",
                "--machine",
                "-t",
                "--type",
                "-p",
                "--property",
                "-n",
                "--lines",
                "-o",
                "--output",
                "--state",
                "-s",
                "--signal",
                "--kill-whom",
                "--root",
            ],
        );
        match ops.first() {
            None => {}
            Some(verb) if READS.contains(verb) => {}
            Some(verb) if POWER.contains(verb) => {
                self.raise(
                    Risk::L3,
                    format!("systemctl {verb}: stops or reboots the server"),
                );
            }
            Some(verb) => {
                let units = ops[1..].join(" ");
                let what = format!("systemctl {verb} {units}");
                if args.contains(&"--user") {
                    self.raise(
                        Risk::L1,
                        format!("{}: changes user services", what.trim_end()),
                    );
                } else {
                    self.raise(
                        Risk::L2,
                        format!("{}: changes system services", what.trim_end()),
                    );
                }
            }
        }
    }

    fn docker(&mut self, name: &str, args: &[&str], depth: usize) {
        let rest = skip_options(
            args,
            &[
                "-H",
                "--host",
                "--context",
                "-c",
                "--config",
                "-l",
                "--log-level",
            ],
        );
        let Some((verb, verb_args)) = rest.split_first() else {
            return;
        };
        match *verb {
            "compose" => self.compose(&format!("{name} compose"), verb_args, depth),
            "container" | "image" | "network" | "volume" | "system" | "builder" | "context"
            | "plugin" | "buildx" | "node" | "service" | "stack" | "secret" | "config"
            | "swarm" | "manifest" | "trust" => {
                let Some((sub, sub_args)) = verb_args.split_first() else {
                    return;
                };
                const READS: &[&str] = &[
                    "ls", "list", "ps", "inspect", "logs", "top", "stats", "port", "diff",
                    "history", "df", "info", "events", "show", "use",
                ];
                if READS.contains(sub) {
                    return;
                }
                match (*verb, *sub) {
                    ("volume", "rm" | "remove" | "prune")
                    | ("system", "prune")
                    | ("container", "prune")
                    | ("image", "prune") => {
                        self.raise(Risk::L3, format!("{name} {verb} {sub}: deletes data"));
                    }
                    ("container", _) => self.docker_verb(name, sub, sub_args, depth),
                    _ => self.raise(Risk::L2, format!("{name} {verb} {sub}: changes containers")),
                }
            }
            _ => self.docker_verb(name, verb, verb_args, depth),
        }
    }

    fn docker_verb(&mut self, name: &str, verb: &str, args: &[&str], depth: usize) {
        const READS: &[&str] = &[
            "ps", "images", "inspect", "logs", "stats", "top", "version", "info", "port", "diff",
            "history", "events", "search", "ls",
        ];
        match verb {
            _ if READS.contains(&verb) => {}
            "exec" => {
                self.raise(
                    Risk::L1,
                    format!("{name} exec: runs a command in a container"),
                );
                if args.contains(&"--privileged") {
                    self.raise(Risk::L2, "runs with extra privileges in a container");
                }
                let rest = skip_options(
                    args,
                    &[
                        "-e",
                        "--env",
                        "--env-file",
                        "-u",
                        "--user",
                        "-w",
                        "--workdir",
                        "--detach-keys",
                    ],
                );
                if let Some((_, command)) = rest.split_first() {
                    self.words(command, depth + 1, Pipe::default());
                }
            }
            "rm" => {
                if has_flag(args, 'v', &["--volumes"]) {
                    self.raise(
                        Risk::L3,
                        format!("{name} rm -v: removes containers and their volumes"),
                    );
                } else {
                    self.raise(Risk::L2, format!("{name} rm: removes containers"));
                }
            }
            "run" | "create" => {
                self.raise(Risk::L2, format!("{name} {verb}: starts a container"));
                let privileged = args.iter().any(|a| {
                    *a == "--privileged"
                        || a.starts_with("--pid=host")
                        || a.starts_with("-v=/:")
                        || a.starts_with("/:")
                });
                if privileged {
                    self.raise(Risk::L2, "the container gets access to the host");
                }
            }
            _ => self.raise(
                Risk::L2,
                format!("{name} {verb}: changes containers or images"),
            ),
        }
    }

    fn compose(&mut self, name: &str, args: &[&str], depth: usize) {
        let rest = skip_options(
            args,
            &[
                "-f",
                "--file",
                "-p",
                "--project-name",
                "--project-directory",
                "--env-file",
                "--profile",
                "--ansi",
                "--progress",
                "--parallel",
            ],
        );
        let Some((verb, verb_args)) = rest.split_first() else {
            return;
        };
        const READS: &[&str] = &[
            "ps", "logs", "config", "ls", "top", "images", "version", "port", "events", "stats",
        ];
        match *verb {
            _ if READS.contains(verb) => {}
            "down" | "rm" if has_flag(verb_args, 'v', &["--volumes"]) => {
                self.raise(Risk::L3, format!("{name} {verb} -v: deletes volumes"));
            }
            "exec" => {
                self.raise(
                    Risk::L1,
                    format!("{name} exec: runs a command in a container"),
                );
                let rest = skip_options(
                    verb_args,
                    &["-e", "--env", "-u", "--user", "-w", "--workdir", "--index"],
                );
                if let Some((_, command)) = rest.split_first() {
                    self.words(command, depth + 1, Pipe::default());
                }
            }
            _ => self.raise(Risk::L2, format!("{name} {verb}: changes containers")),
        }
    }

    fn git(&mut self, args: &[&str]) {
        let rest = skip_options(
            args,
            &["-C", "-c", "--git-dir", "--work-tree", "--namespace"],
        );
        let Some((verb, args)) = rest.split_first() else {
            return;
        };
        const READS: &[&str] = &[
            "status",
            "log",
            "diff",
            "show",
            "blame",
            "grep",
            "ls-files",
            "ls-tree",
            "ls-remote",
            "rev-parse",
            "rev-list",
            "describe",
            "shortlog",
            "cat-file",
            "reflog",
            "show-ref",
            "whatchanged",
            "version",
            "help",
            "count-objects",
            "for-each-ref",
            "check-ignore",
            "name-rev",
            "merge-base",
            "var",
        ];
        let ops = operands(args, &[]);
        let discard =
            |reason: &str, this: &mut Self| this.raise(Risk::L2, format!("git {verb}: {reason}"));
        match *verb {
            _ if READS.contains(verb) => {}
            "branch" => {
                if has_flag(args, 'D', &[])
                    || args.contains(&"--delete") && args.contains(&"--force")
                {
                    discard("deletes an unmerged branch", self);
                } else if !ops.is_empty()
                    || has_flag(args, 'd', &["--delete"])
                    || has_flag(args, 'm', &["--move"])
                {
                    self.raise(Risk::L1, "git branch: changes branches");
                }
            }
            "tag" | "remote" => {
                let reads = ops.is_empty()
                    || has_flag(args, 'l', &["--list"])
                    || matches!(ops.first(), Some(&"show" | &"get-url"));
                if !reads {
                    self.raise(Risk::L1, format!("git {verb}: changes the repository"));
                }
            }
            "config" => {
                let reads = args
                    .iter()
                    .any(|a| a.starts_with("--get") || matches!(*a, "--list" | "-l"))
                    || ops.len() == 1 && !args.iter().any(|a| a.starts_with("--unset"));
                if !reads {
                    self.raise(Risk::L1, "git config: changes settings");
                }
            }
            "stash" => match ops.first() {
                Some(&"list" | &"show") => {}
                Some(&"drop" | &"clear") => discard("discards stashed changes", self),
                _ => self.raise(Risk::L1, "git stash: changes the working tree"),
            },
            "push" => {
                let force = has_flag(args, 'f', &["--force", "--mirror", "--delete", "--prune"])
                    || args.iter().any(|a| a.starts_with("--force"))
                    || ops
                        .iter()
                        .skip(1)
                        .any(|r| r.starts_with('+') || r.starts_with(':'));
                if force {
                    discard("rewrites or deletes remote history", self);
                } else {
                    self.raise(Risk::L1, "git push: publishes commits");
                }
            }
            "reset" if args.contains(&"--hard") || args.contains(&"--merge") => {
                discard("discards uncommitted changes", self);
            }
            "clean" => {
                if has_flag(args, 'f', &["--force"]) {
                    discard("deletes untracked files", self);
                }
            }
            "checkout" | "switch" => {
                if has_flag(args, 'f', &["--force", "--discard-changes"])
                    || args.contains(&"--")
                    || ops.contains(&".")
                {
                    discard("discards uncommitted changes", self);
                } else {
                    self.raise(Risk::L1, format!("git {verb}: changes the working tree"));
                }
            }
            "restore" => {
                if args.contains(&"--staged") && !args.contains(&"--worktree") {
                    self.raise(Risk::L1, "git restore --staged: unstages changes");
                } else {
                    discard("discards uncommitted changes", self);
                }
            }
            "filter-branch" | "filter-repo" | "update-ref" => discard("rewrites history", self),
            _ => self.raise(Risk::L1, format!("git {verb}: changes the repository")),
        }
    }

    fn packages(&mut self, name: &str, args: &[&str]) {
        const READS: &[&str] = &[
            "list",
            "search",
            "show",
            "info",
            "policy",
            "madison",
            "depends",
            "rdepends",
            "history",
            "check-update",
            "provides",
            "whatprovides",
            "repolist",
            "changelog",
            "list-installed",
            "query",
            "find",
            "contains",
            "files",
            "dotty",
            "stats",
            "check",
        ];
        let reads = match name {
            "dpkg" => [
                "-l",
                "-L",
                "-s",
                "-S",
                "-p",
                "--list",
                "--status",
                "--listfiles",
                "--search",
                "--get-selections",
                "--print-architecture",
                "--compare-versions",
            ]
            .iter()
            .any(|flag| args.contains(flag)),
            "rpm" => has_flag(args, 'q', &["--query", "--verify"]) || has_flag(args, 'V', &[]),
            "pacman" => args.first().is_some_and(|a| {
                a.starts_with("-Q")
                    || a.starts_with("-S")
                        && (a.contains('s') || a.contains('i') || a.contains('l'))
                        && !a.contains('y')
                        && !a.contains('u')
            }),
            _ => operands(args, &["-o", "-c", "-t", "--target-release", "--option"])
                .first()
                .is_some_and(|verb| READS.contains(verb)),
        };
        if !reads {
            self.raise(Risk::L2, format!("{name}: changes system packages"));
        }
    }

    fn database(&mut self, name: &str, args: &[&str]) {
        let query = match name {
            "psql" => option_values(args, Some('c'), &["--command"]),
            "sqlite3" => {
                let ops = operands(args, &["-cmd", "-separator", "-nullvalue", "-init"]);
                ops.get(1..)
                    .map(|rest| vec![rest.join(" ")])
                    .unwrap_or_default()
            }
            "clickhouse-client" | "clickhouse" => option_values(args, Some('q'), &["--query"]),
            _ => option_values(args, Some('e'), &["--execute"]),
        };
        if query.is_empty() {
            self.raise(Risk::L1, format!("{name}: runs database commands"));
            return;
        }
        for sql in query {
            self.sql(name, &sql);
        }
    }

    fn sql(&mut self, name: &str, sql: &str) {
        let upper = sql.to_ascii_uppercase();
        let words: Vec<&str> = upper
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .collect();
        let has = |word: &str| words.contains(&word);
        if has("DROP") || has("TRUNCATE") {
            self.raise(
                Risk::L3,
                format!("{name}: drops or truncates database objects"),
            );
        } else if has("DELETE") && !has("WHERE") {
            self.raise(Risk::L3, format!("{name}: deletes every row of a table"));
        } else if [
            "INSERT", "UPDATE", "DELETE", "ALTER", "CREATE", "GRANT", "REVOKE", "RENAME",
            "REPLACE", "MERGE", "COPY", "VACUUM", "REINDEX", "CLUSTER", "LOAD", "FLUSH", "KILL",
            "OPTIMIZE", "ATTACH", "DETACH", "IMPORT",
        ]
        .iter()
        .any(|w| has(w))
        {
            self.raise(Risk::L2, format!("{name}: changes the database"));
        }
    }

    fn redis(&mut self, args: &[&str]) {
        let ops = operands(
            args,
            &[
                "-h", "-p", "-a", "-n", "-u", "--user", "--pass", "-s", "-r", "-i", "--cacert",
                "--cert", "--key",
            ],
        );
        let Some(command) = ops.first().map(|c| c.to_ascii_uppercase()) else {
            if !args.iter().any(|a| {
                matches!(
                    *a,
                    "--stat" | "--bigkeys" | "--memkeys" | "--scan" | "--latency"
                )
            }) {
                self.raise(Risk::L1, "redis-cli: opens a redis shell");
            }
            return;
        };
        const READS: &[&str] = &[
            "GET",
            "MGET",
            "KEYS",
            "SCAN",
            "HGET",
            "HMGET",
            "HGETALL",
            "HKEYS",
            "HVALS",
            "HLEN",
            "HEXISTS",
            "HSCAN",
            "LRANGE",
            "LLEN",
            "LINDEX",
            "SMEMBERS",
            "SCARD",
            "SISMEMBER",
            "SSCAN",
            "ZRANGE",
            "ZRANGEBYSCORE",
            "ZREVRANGE",
            "ZCARD",
            "ZSCORE",
            "ZRANK",
            "ZSCAN",
            "TTL",
            "PTTL",
            "TYPE",
            "EXISTS",
            "STRLEN",
            "INFO",
            "PING",
            "DBSIZE",
            "MONITOR",
            "SLOWLOG",
            "MEMORY",
            "OBJECT",
            "XRANGE",
            "XREVRANGE",
            "XLEN",
            "XINFO",
            "LATENCY",
            "ROLE",
            "TIME",
            "ECHO",
            "LASTSAVE",
            "COMMAND",
            "PUBSUB",
        ];
        let second = ops.get(1).map(|s| s.to_ascii_uppercase());
        match command.as_str() {
            "FLUSHALL" | "FLUSHDB" | "SHUTDOWN" | "DEBUG" => {
                self.raise(
                    Risk::L3,
                    format!("redis-cli {command}: deletes data or stops redis"),
                );
            }
            "CONFIG" | "CLIENT" | "CLUSTER" | "ACL"
                if matches!(
                    second.as_deref(),
                    Some("GET" | "LIST" | "INFO" | "NODES" | "WHOAMI" | "USERS")
                ) => {}
            _ if READS.contains(&command.as_str()) => {}
            _ => self.raise(
                Risk::L2,
                format!("redis-cli {command}: changes data in redis"),
            ),
        }
    }
}

fn path_risk(path: &str) -> (Risk, String) {
    if is_block_device(path) {
        (Risk::L3, format!("writes to the device {path}"))
    } else if path.starts_with("/dev/") {
        (Risk::L2, format!("writes to the device {path}"))
    } else if is_system(path) {
        (Risk::L2, format!("{path} is a system path"))
    } else {
        (Risk::L1, format!("writes {path}"))
    }
}

fn is_system(path: &str) -> bool {
    path == "/"
        || SYSTEM_DIRS.iter().any(|dir| {
            path == *dir
                || path
                    .strip_prefix(dir)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

fn is_harmless_device(path: &str) -> bool {
    matches!(
        path,
        "/dev/null" | "/dev/stdout" | "/dev/stderr" | "/dev/stdin" | "/dev/tty" | "/dev/zero"
    ) || path.starts_with("/dev/fd/")
        || path.starts_with("/dev/shm/")
}

fn is_block_device(path: &str) -> bool {
    let Some(name) = path.strip_prefix("/dev/") else {
        return false;
    };
    [
        "sd", "hd", "vd", "xvd", "nvme", "mmcblk", "md", "dm-", "disk", "mapper/", "loop", "sr",
        "nbd", "rbd", "zd",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn is_disk_tool(name: &str) -> bool {
    matches!(
        name,
        "mke2fs"
            | "mkswap"
            | "wipefs"
            | "fdisk"
            | "sfdisk"
            | "gdisk"
            | "sgdisk"
            | "cfdisk"
            | "parted"
            | "blkdiscard"
            | "pvremove"
            | "vgremove"
            | "lvremove"
            | "zpool"
            | "mdadm"
            | "cryptsetup"
            | "e2fsck"
            | "fsck"
            | "xfs_repair"
            | "badblocks"
    )
}

/// A delete target that would remove far more than one project: `/`, a top-level
/// directory, the home directory, `..`, or a path the shell expands.
fn is_broad(target: &str, resolved: &str) -> bool {
    if target.contains('$') || target.contains('`') {
        return true;
    }
    if matches!(
        target,
        "/" | "." | ".." | "./" | "../" | "*" | ".*" | "~" | "~/" | "~/*" | "/*" | "./*"
    ) {
        return true;
    }
    if target.starts_with("..") || target.contains("/../") || target.ends_with("/..") {
        return true;
    }
    if resolved.starts_with('/') {
        let depth = resolved.split('/').filter(|c| !c.is_empty()).count();
        return depth <= 2;
    }
    false
}

/// Files that commonly hold credentials or private keys.
fn is_secret(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(
        path,
        "/etc/shadow" | "/etc/gshadow" | "/etc/sudoers" | "/etc/master.passwd"
    ) {
        return true;
    }
    if path.contains("/.ssh/") || path.starts_with(".ssh/") {
        let key = name.starts_with("id_") && !name.ends_with(".pub");
        if key || name.ends_with("_key") {
            return true;
        }
    }
    if path.contains("/etc/ssl/private/") {
        return true;
    }
    name == ".env"
        || name.starts_with(".env.") && !name.ends_with(".example") && !name.ends_with(".sample")
        || [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"]
            .iter()
            .any(|ext| name.ends_with(ext))
        || matches!(name, ".netrc" | ".pgpass" | ".my.cnf")
        || path.ends_with(".aws/credentials")
        || path.ends_with(".docker/config.json")
        || path.ends_with(".kube/config")
}

fn looks_like_path(word: &str) -> bool {
    word.starts_with('/')
        || word.starts_with("~/")
        || word.starts_with("./")
        || word.starts_with("../")
}

fn is_remote_spec(target: &str) -> bool {
    match target.split_once(':') {
        Some((host, _)) => !host.is_empty() && !host.contains('/'),
        None => false,
    }
}

/// Collapses `.`, `..` and repeated slashes in an absolute path.
pub fn normalize(path: &str) -> String {
    if !path.starts_with('/') {
        return path.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

fn program_name(word: &str) -> &str {
    let word = word.trim_start_matches('\\');
    word.rsplit('/').next().unwrap_or(word)
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// The arguments after the leading options; `with_values` lists options followed by a
/// separate value.
fn skip_options<'a, 'b>(args: &'a [&'b str], with_values: &[&str]) -> &'a [&'b str] {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            return &args[index + 1..];
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        index += if with_values.contains(&arg) { 2 } else { 1 };
    }
    args.get(index..).unwrap_or_default()
}

/// Non-option arguments anywhere in `args` (GNU style); `with_values` lists options
/// followed by a separate value.
fn operands<'a>(args: &[&'a str], with_values: &[&str]) -> Vec<&'a str> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            result.extend_from_slice(&args[index + 1..]);
            break;
        }
        if arg.starts_with('-') && arg != "-" {
            index += if with_values.contains(&arg) { 2 } else { 1 };
            continue;
        }
        result.push(arg);
        index += 1;
    }
    result
}

/// `-x` alone or in a cluster such as `-rf`, or one of the long forms (also `--long=…`).
fn has_flag(args: &[&str], short: char, long: &[&str]) -> bool {
    for arg in args {
        if *arg == "--" {
            break;
        }
        if let Some(name) = arg.strip_prefix("--") {
            let name = name.split('=').next().unwrap_or(name);
            if long.iter().any(|l| l.trim_start_matches("--") == name) {
                return true;
            }
        } else if let Some(cluster) = arg.strip_prefix('-')
            && !cluster.is_empty()
            && !cluster.starts_with(|c: char| c.is_ascii_digit())
            && cluster.chars().all(|c| c.is_ascii_alphanumeric())
            && cluster.contains(short)
        {
            return true;
        }
    }
    false
}

/// Values of an option: `-o v`, `-ov`, `-xo v` (last in a cluster), `--long v`, `--long=v`.
fn option_values(args: &[&str], short: Option<char>, long: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            break;
        }
        if let Some(name) = arg.strip_prefix("--") {
            let (name, inline) = match name.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (name, None),
            };
            if long.iter().any(|l| l.trim_start_matches("--") == name) {
                match inline {
                    Some(value) => values.push(value.to_string()),
                    None => {
                        if let Some(value) = args.get(index + 1) {
                            values.push(value.to_string());
                        }
                        index += 1;
                    }
                }
            }
        } else if let (Some(short), Some(cluster)) = (short, arg.strip_prefix('-'))
            && let Some(position) = cluster.find(short)
            && cluster[..position].chars().all(|c| c.is_ascii_alphabetic())
        {
            let rest = &cluster[position + short.len_utf8()..];
            if rest.is_empty() {
                if let Some(value) = args.get(index + 1) {
                    values.push(value.to_string());
                }
                index += 1;
            } else {
                values.push(rest.to_string());
            }
        }
        index += 1;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn risk(line: &str) -> Risk {
        command(line, None, None).risk
    }

    #[test]
    fn read_only_commands_are_l0() {
        for line in [
            "ls -la /var/www",
            "cat /etc/nginx/nginx.conf | grep server_name",
            "ps aux | grep node | head -5",
            "df -h && free -m; uptime",
            "systemctl status nginx",
            "docker ps -a",
            "docker compose logs --tail 100 web",
            "git status && git log --oneline -5",
            "journalctl -u app --since today",
            "curl -s http://127.0.0.1:8080/health",
            "wget -qO- http://localhost/",
            "echo hi > /dev/null 2>&1",
            "tar -tzf release.tar.gz",
            "mysql -e 'SELECT count(*) FROM users'",
            "redis-cli -n 1 GET session:1",
            "apt list --installed",
            "sudo --version",
            "find /var/log -name '*.gz' -mtime +7",
            "FOO=1 env | sort",
        ] {
            assert_eq!(risk(line), Risk::L0, "{line}");
        }
    }

    #[test]
    fn user_changes_are_l1() {
        for line in [
            "mkdir -p ~/app && cd ~/app && git pull",
            "npm install",
            "echo hello > notes.txt",
            "rm -rf node_modules",
            "rm build.log",
            "./deploy.sh",
            "python3 manage.py migrate",
            "cat .env",
            "kill 4242",
            "curl -X POST http://127.0.0.1:8080/reload",
            "sed -i 's/a/b/' app.conf",
            "pm2 restart api",
            "systemctl --user restart app",
            "docker exec web ls /app",
        ] {
            assert_eq!(risk(line), Risk::L1, "{line}");
        }
    }

    #[test]
    fn system_changes_are_l2() {
        for line in [
            "sudo systemctl restart nginx",
            "systemctl restart nginx",
            "apt-get install -y nginx",
            "docker compose up -d",
            "docker run -d -p 80:80 nginx",
            "echo 'x' >> /etc/hosts",
            "cp app.conf /etc/nginx/conf.d/",
            "sudo -u postgres psql",
            "ufw allow 443",
            "git reset --hard origin/main",
            "rm -rf /var/www/app/releases/41",
            "cat script.sh | bash",
            "docker exec web rm -rf /app/var/cache",
            "mysql -e 'UPDATE users SET admin=1 WHERE id=1'",
            "echo 'oops",
            "crontab -r",
            "tee /etc/motd <<< hi",
        ] {
            assert_eq!(risk(line), Risk::L2, "{line}");
        }
    }

    #[test]
    fn destructive_commands_are_l3() {
        for line in [
            "rm -rf /",
            "rm -rf /*",
            "sudo rm -rf /var/www",
            "rm -fr ~",
            "rm -rf $DIR/",
            "rm -rf \"$(pwd)\"/..",
            "cd /etc && rm -rf nginx",
            "mkfs.ext4 /dev/sdb1",
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "reboot",
            "sudo shutdown -h now",
            "systemctl poweroff",
            "curl -fsSL https://example.com/install.sh | sh",
            "wget -qO- https://x.io/i | sudo bash",
            "psql -c 'DROP TABLE users'",
            "mysql -e 'delete from orders'",
            "docker system prune -af",
            "docker compose down -v",
            "redis-cli FLUSHALL",
            "bash -c 'rm -rf /home'",
            "echo $(rm -rf /srv)",
            "find / -name '*.log' -delete",
            "chmod -R 777 /",
            "kill -9 -1",
            "echo bad > /dev/sda",
            "sh -c \"sudo sh -c 'rm -rf /etc'\"",
            "nohup timeout 10 sudo -E env A=1 rm -rf /usr &",
        ] {
            assert_eq!(risk(line), Risk::L3, "{line}");
        }
    }

    #[test]
    fn cwd_and_stdin_are_taken_into_account() {
        assert_eq!(command("rm -rf nginx", Some("/etc"), None).risk, Risk::L3);
        assert_eq!(
            command("echo x > app.conf", Some("/etc"), None).risk,
            Risk::L2
        );
        assert_eq!(command("bash", None, Some("rm -rf /")).risk, Risk::L3);
        assert_eq!(command("bash -s", None, Some("ls")).risk, Risk::L0);
    }

    #[test]
    fn reasons_and_paths_are_reported() {
        let assessment = command("sudo systemctl restart nginx && rm -rf /", None, None);
        assert_eq!(assessment.risk, Risk::L3);
        assert_eq!(assessment.reasons()[0], "rm -r on /");
        assert!(assessment.reasons().iter().any(|r| r.contains("sudo")));
        assert!(assessment.paths.contains(&"/".to_string()));

        let write = command("cat a > /etc/app/../app.conf", None, None);
        assert_eq!(write.paths, vec!["/etc/app.conf"]);
    }

    #[test]
    fn file_targets() {
        assert_eq!(file_write("/home/app/config.yml").risk, Risk::L1);
        assert_eq!(file_write("/etc/nginx/nginx.conf").risk, Risk::L2);
        assert_eq!(file_write("/etc/../root/.bashrc").risk, Risk::L2);
        assert_eq!(file_read("/srv/app/.env").risk, Risk::L1);
        assert_eq!(file_read("/srv/app/README.md").risk, Risk::L0);
        assert_eq!(file_read("/home/u/.ssh/id_ed25519").risk, Risk::L1);
        assert_eq!(file_read("/home/u/.ssh/id_ed25519.pub").risk, Risk::L0);
    }

    #[test]
    fn option_helpers() {
        assert!(has_flag(&["-rf", "x"], 'r', &[]));
        assert!(has_flag(&["--recursive"], 'r', &["--recursive"]));
        assert!(!has_flag(&["-9"], 'r', &[]));
        assert_eq!(
            option_values(&["-sSLo", "out.txt", "url"], Some('o'), &[]),
            vec!["out.txt"]
        );
        assert_eq!(option_values(&["-qO-", "url"], Some('O'), &[]), vec!["-"]);
        assert_eq!(
            option_values(&["--output=a"], None, &["--output"]),
            vec!["a"]
        );
        assert_eq!(normalize("/var//www/./a/../b/"), "/var/www/b");
    }
}
