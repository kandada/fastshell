# fastshell — capability matrix & known limitations

Status snapshot of the in-process shell engine (mobile-first, no external binaries).

## Engines / runtimes
| Area | Support | Notes |
|------|---------|-------|
| Shell language | broad | params, arithmetic (hex/base/`**`), indexed + associative arrays, `case`/functions/subshells, heredocs, here-strings (`$'…'`), process substitution `<()`, `sh -c` isolation |
| Globbing | deep | `*`/`?`/`[]`, `**`, `extglob` (`@?*+!()`), `nullglob`/`failglob`/`nocaseglob`/`dotglob` |
| Job control | synchronous | `&` records a job; `$!`/`jobs`/`wait`/`kill %n`/`disown` work; **no true concurrency** |
| Python | embedded | RustPython or host CPython; sandboxed FS wrapper; `-c`/`-m`/script/stdin; **networking via `_socket`/`_ssl`** (`socket`, `http.client`, `urllib`, HTTPS); `pip` install/list/freeze/show/uninstall/-r |
| JS | host WebView | `node`/`js` execute; **Promises and timers are driven** (`await`/`.then`/`setTimeout`/`setInterval`, drained up to 3s) and `console.*` is captured; no `require`/modules/`process` |
| Network | curl/wget/ping/dig | permission-gated; deadline-bounded (no wedge); `--data-urlencode`/`--compressed`/`--proxy`/multipart/cookies |
| VCS | git (libgit2) | clone/pull/push/commit/status/log/diff/branch/checkout/merge/stash/remote/reset |

## Commands
Coreutils-like: text (`grep -A/-B/-C`, `sed` incl. step addresses/`-z`/`a/i/c`/`s///p`, `awk` incl. user functions + `getline`(file form) + `split` arrays, `sort -h/-V`, `uniq -f/-s/-w`, `xargs -0/-P/-n`), files (`find` incl. `-size/-perm/-newer/-newermt/-empty/-delete/-printf`), archives (`tar` auto-detect gz/xz/bz2/zstd + `--strip-components`, `zip`/`unzip -d`, `gzip`/`bzip2`/`xz`/`zstd`, `cpio` newc), hashes (`md5/sha1/sha224/256/384/512/sha3/cksum/crc32/b2sum`), encoding (`base64`/`base32`), archiving, `jq` (incl. `//`, `reduce`, `foreach`), `bc` (incl. `ibase`/`obase`/`scale`), `sqlite3` (incl. `.dump/.schema/.import`), plus `curl`/`wget`.

## Known limitations (by design / engine-bound)
- **No true async jobs** — single `Runtime` behind a mutex; `&` runs synchronously (`&`/`$!` are synthetic; no real processes).
- **JS modules** — no `require`/ESM/`process`; only what the host WebView exposes. Timers are drained for up to 3s, so an `setInterval` never signals completion and is cut off at the cap.
- **Python networking** — TCP/TLS/HTTP(S) work (`_socket`/`_ssl`); **`pip-install` installs only the named package and does not resolve dependencies** (install deps explicitly). `subprocess` spawning is blocked by the sandbox; `ctypes` is unavailable.
- **ICMP `ping`** — raw ICMP is unavailable in the sandbox; `ping` falls back to a TCP-connect probe over common ports (443/80/22/7).
- **`render` external assets** — external `http(s)` images/styles/scripts and CSS `url(...)` are fetched and inlined before rendering; anything unreachable is left as-is with a warning.
- **trap** — `EXIT` and `ERR` fire; `INT` fires on host cancel; `TERM` is not delivered.

## Tests
`tests/session_fixes.rs`, `tests/foundation_hardening.rs`, `tests/long_tail.rs`, plus lib unit tests.
