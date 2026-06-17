# fastshell Command Reference

All built-in shell commands. Commands marked with `*` require DevicePlugin registration.

## File Operations

| Command | Flags | Description |
|---------|-------|-------------|
| `ls` | `-l` long format, `-a` all, `-h` human sizes, `-R` recursive | List directory |
| `cd` | | Change working directory |
| `pwd` | | Print working directory |
| `mkdir` | `-p` create parents | Create directory |
| `rmdir` | | Remove empty directory |
| `rm` | `-r` recursive, `-f` force | Remove files/directories |
| `cp` | `-r` recursive | Copy files/directories |
| `mv` | | Move/rename files |
| `touch` | | Create file or update timestamp |
| `ln` | `-s` symbolic | Create link |
| `readlink` | `-f` canonicalize | Print symlink target |
| `chmod` | `-R` recursive, octal (755) or symbolic (+x, u+x, go-w) | Change file permissions |
| `chown` | | Change owner |
| `chgrp` | | Change group |
| `stat` | `-c fmt` custom format (%n %s %a %A %F...) | File metadata |
| `file` | | Detect file type |
| `du` | `-h` human, `-s` summary | Disk usage |
| `df` | `-h` human | Disk free |
| `basename` | | Strip directory from path |
| `dirname` | | Strip filename from path |
| `realpath` | | Resolve to absolute path |
| `truncate` | `-s SIZE` | Shrink/extend file |
| `mktemp` | `-d` directory | Create temp file/dir |
| `install` | | Copy + set attributes |

## Text Processing

| Command | Flags | Description |
|---------|-------|-------------|
| `cat` | `-n` line numbers | Concatenate files |
| `grep` | `-i` case-insensitive, `-v` invert, `-c` count, `-n` line numbers, `-r` recursive, `-l` files-with-matches, `-o` only-matching, `-w` word, `-q` quiet, `-E` extended regex, `-F` fixed strings | Search text |
| `sed` | `-i` in-place, `-e` script, `-n` quiet, range addr (1,5 / /pat1/,/pat2/), capture groups (\1), ! negation, a/i/c commands | Stream editor |
| `awk` | `-F sep` field sep, `-v var=val`, BEGIN/END, printf, length(), substr(), split(), tolower(), toupper(), next, &&/|| | Pattern scanning |
| `sort` | `-n` numeric, `-r` reverse, `-u` unique, `-k N,M` column, `-t CHAR` delimiter, `-h` human-num, `-f` ignore-case, `-s` stable | Sort lines |
| `uniq` | `-c` count, `-d` duplicates, `-u` unique, `-i` case, `-f N` skip fields | Filter repeated lines |
| `wc` | `-l` lines, `-w` words, `-c` chars | Count |
| `head` | `-n N` lines, `-c N` bytes | Output first part |
| `tail` | `-n N` lines, `-c N` bytes | Output last part |
| `cut` | `-d CHAR` delimiter, `-f LIST` fields, `-c LIST` chars, `--complement` | Select columns |
| `tr` | `-d` delete, `-s` squeeze, `-c` complement, [:alpha:] [:digit:] classes | Translate characters |
| `diff` | `-u` unified, `-r` recursive, `-b`/`-w` whitespace, `-q` brief, `-U N` context | Compare files |
| `tee` | `-a` append | Read stdin, write to stdout + files |
| `xargs` | `-n N` max-args, `-I {}` replace, `-0` null, `-P N` parallel, `-t` verbose | Build commands from stdin |
| `paste` | | Merge lines of files |
| `comm` | | Compare sorted files |
| `rev` | | Reverse lines |
| `tac` | | Reverse lines (cat backwards) |
| `nl` | | Number lines |
| `fold` | `-w N` width | Wrap lines |
| `expand` | | Tabs to spaces |
| `unexpand` | | Spaces to tabs |
| `shuf` | | Shuffle lines |
| `split` | `-l N` lines, `-b N` bytes | Split files |
| `strings` | | Print printable chars |
| `printf` | | Format and print |
| `echo` | `-n` no newline | Print text |
| `column` | `-t` table, `-s` separator | Format columns |

## Network

| Command | Flags | Description |
|---------|-------|-------------|
| `curl` | `-o FILE`, `-O`, `-L`, `-s`, `-X METHOD`, `-d DATA`, `-H HEADER`, `-I` HEAD, `-u user:pass`, `-k` insecure, `-v` verbose, `-w fmt` | HTTP client |
| `wget` | `-O FILE` | Download files |
| `ping` | `-c N`, `-W SEC`, `-q` | TCP connectivity test |
| `ssh` | `-p PORT`, `-i KEYFILE`, `user@host [command]` | SSH client |
| `nslookup` | | DNS lookup |
| `whois` | | Domain whois |
| `dig` | | DNS query |
| `ifconfig` | | Network interfaces |
| `netstat` | | Network stats |
| `nc` | | Netcat |
| `telnet` | | Telnet client |
| `traceroute` | | Trace route |
| `ss` | | Socket stats |
| `ip` | | IP routing |

## Compression

| Command | Flags | Description |
|---------|-------|-------------|
| `tar` | `-c` create, `-x` extract, `-t` list, `-f FILE`, `-z` gzip, `-v` verbose, `-C DIR` | Archive |
| `gzip` | `-c` stdout, `-d` decompress, `-k` keep, `-1..-9` level | Compress |
| `gunzip` | `-c` stdout, `-k` keep | Decompress |
| `zip` | `-r` recursive | Create zip |
| `unzip` | | Extract zip |
| `bzip2` | | Bzip2 compress |
| `bunzip2` | | Bzip2 decompress |
| `xz` | | XZ compress |
| `unxz` | | XZ decompress |
| `zcat` | | Decompress to stdout |

## Crypto / Encoding

| Command | Flags | Description |
|---------|-------|-------------|
| `base64` | `-d` decode, `-w N` wrap | Base64 encode/decode |
| `sha256sum` | `-c FILE` check | SHA-256 |
| `sha512sum` | `-c FILE` check | SHA-512 |
| `md5sum` / `md5` | | MD5 |
| `sha1sum` | | SHA-1 |
| `sha3sum` | | SHA-3 |
| `sum` | | BSD checksum |
| `xxd` | `-r` reverse, `-p` plain, `-l N` limit, `-s N` seek | Hex dump |
| `hexdump` | | Hex dump (alt) |
| `od` | | Octal dump |
| `uuidgen` | | Generate UUID |

## JSON

| Command | Flags | Description |
|---------|-------|-------------|
| `jq` | `-r` raw, `-c` compact, `-s` slurp, select/map/keys/length/if/+-*/() | JSON processor |

## System

| Command | Flags | Description |
|---------|-------|-------------|
| `ps` | `-o fmt`, `-p PID`, `-u USER` | Process list |
| `kill` | `-s SIG`, `-l` list signals | Send signal |
| `killall` | | Kill by name |
| `pgrep` | | Search processes |
| `pkill` | | Signal by name |
| `pidof` | | Get PID by name |
| `pstree` | | Process tree |
| `nice` | | Set priority |
| `renice` | | Change priority |
| `nohup` | | Immune to hangups |
| `env` | | Print environment |
| `printenv` | | Print env var |
| `date` | `+format`, `-u` UTC, `-d STR` parse | Print/set date |
| `sleep` | SECONDS | Delay |
| `timeout` | SECONDS CMD | Run with timeout |
| `watch` | `-n SEC` | Run periodically |
| `which` | | Locate command |
| `uname` | `-a` all, `-s` kernel | System info |
| `hostname` | | Host name |
| `whoami` | | Current user |
| `id` | | User/group IDs |
| `groups` | | Group memberships |
| `uptime` | | System uptime |
| `free` | `-h` human | Memory usage |
| `sync` | | Flush filesystem |
| `clear` | | Clear terminal |
| `reset` | | Reset terminal |
| `logger` | | Syslog message |
| `dmesg` | | Kernel messages |
| `tty` | | Terminal name |
| `nproc` | | CPU count |
| `hostid` | | Host ID |
| `logname` | | Login name |
| `who` | | Logged in users |

## Control Flow

| Command | Flags | Description |
|---------|-------|-------------|
| `true` | | Return 0 |
| `false` | | Return 1 |
| `test` / `[` | `-f` `-d` `-eq` `-ne` `-lt` `-gt` `=` `!=` `-z` `-n` `&&` `||` `!` | Conditional |
| `expr` | `+` `-` `*` `/` `%` | Arithmetic |
| `yes` | | Repeated output |
| `seq` | FIRST [INCR] LAST | Number sequence |

## Version Control

| Command | Flags | Description |
|---------|-------|-------------|
| `git` | init, clone, status, add, commit, push, pull, log, diff, checkout, branch | Git operations |

## Device (\* requires plugin)

| Command | Flags | Description |
|---------|-------|-------------|
| `camera` \* | `[output_path]` | Take photo |
| `screencapture` \* | `[output_path]` | Screenshot |
| `photolib` \* | `-n N --video -o DIR` | Pick from photo library |
| `record` / `arecord` \* | `-d SEC -o FILE` | Record audio |
| `play` \* | FILE | Play audio |
| `say` \* | TEXT | Text to speech |
| `speech` \* | FILE | Speech to text |
| `contacts` \* | `get ID` `search NAME` `-n N` | Contacts |
| `location` \* | | GPS location |
| `clipboard` / `pbpaste` \* | `set TEXT` | Clipboard |
| `pbcopy` \* | TEXT | Write to clipboard |
| `sensor` \* | `orientation motion light proximity list` | Device sensors |
| `notify` / `notify-send` \* | TITLE BODY `--sound` | Send notification |
| `share` \* | FILE `--text TEXT` | Share via system dialog |
| `open` / `xdg-open` \* | URL/FILE | Open with system app |
| `auth` \* | `bio [reason]` | Biometric auth |
| `battery` \* | | Battery info |
| `vibrate` \* | [MS] | Vibrate device |
| `screen` \* | `brightness 0-1` `on` `off` | Screen control |
| `device` \* | `info` `network` | Device metadata |
