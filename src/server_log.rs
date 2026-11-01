//! Error logs on servers (PHP `error_log`, Apache, nginx): lines are read as
//! they are appended and turned into entries with file and line, so errors
//! show up at the right place in the editor.

/// One error from a log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Time as written in the log (`01-Nov-2026 12:34:56 UTC`), if any.
    pub time: Option<String>,
    /// 1 error, 2 warning, 3 notice (like [`crate::editor::document::Diagnostic`]).
    pub severity: u8,
    /// The message without time and server prefixes.
    pub message: String,
    /// Path on the server as the log names it, and the line (1-based).
    pub file: Option<(String, usize)>,
}

/// Parses one log line; `None` for blank lines and stack trace continuations.
pub fn parse_line(line: &str) -> Option<LogEntry> {
    let line = line.trim_end();
    let trimmed = line.trim_start();
    // Stack traces and the like belong to the entry above.
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("Stack trace:") || trimmed.starts_with("thrown in ") || trimmed.starts_with("Next ") {
        return None;
    }
    let (time, rest) = leading_time(line);
    let message = clean_message(rest);
    if message.is_empty() {
        return None;
    }
    Some(LogEntry { time, severity: severity(&message, rest), file: file_and_line(&message), message })
}

/// Parses a chunk of a log (whole lines only).
pub fn parse(text: &str) -> Vec<LogEntry> {
    text.lines().filter_map(parse_line).collect()
}

/// `[01-Nov-2026 12:34:56 UTC] …` (PHP) or `[Sun Nov 01 12:34:56.123 2026] …`
/// (Apache), or nginx `2026/11/01 12:34:56 [error] …`.
fn leading_time(line: &str) -> (Option<String>, &str) {
    if let Some(rest) = line.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let inner = &rest[..end];
        if inner.chars().any(|c| c.is_ascii_digit()) && inner.contains(':') {
            return (Some(inner.to_string()), rest[end + 1..].trim_start());
        }
    }
    let b = line.as_bytes();
    if b.len() > 19 && b[4] == b'/' && b[7] == b'/' && b[13] == b':' {
        return (Some(line[..19].to_string()), line[19..].trim_start());
    }
    (None, line)
}

/// Drops `[php:error] [pid 1] [client 1.2.3.4:5]`-style prefixes, nginx
/// wrappers and trailing `, referer: …` / `, client: …` parts.
fn clean_message(rest: &str) -> String {
    let mut s = rest;
    // nginx with PHP-FPM: `… FastCGI sent in stderr: "PHP message: PHP Warning: …" while reading …`
    if let Some(i) = s.find("PHP message: ") {
        s = &s[i + "PHP message: ".len()..];
        if let Some(end) = s.find("\" while ").or_else(|| s.find("\"; ")).or_else(|| s.find("PHP message: ")) {
            s = &s[..end];
        }
    }
    loop {
        let t = s.trim_start();
        // `[php:error]`, `[pid 1234]`, `[client …]`, nginx `[error]`.
        if t.starts_with('[')
            && let Some(end) = t.find(']')
            && (!t[1..end].contains(' ') || t.starts_with("[pid ") || t.starts_with("[client "))
        {
            s = &t[end + 1..];
            continue;
        }
        // nginx: `123#0: *45 `
        if let Some(colon) = t.find(": ")
            && let Some(head) = t.get(..colon)
            && head.contains('#')
            && head.chars().all(|c| c.is_ascii_digit() || c == '#' || c == '*' || c == ' ')
        {
            s = &t[colon + 2..];
            continue;
        }
        s = t;
        break;
    }
    let mut s = s.to_string();
    for tail in [", referer: ", ", client: "] {
        if let Some(i) = s.find(tail) {
            s.truncate(i);
        }
    }
    s.trim().to_string()
}

fn severity(message: &str, line: &str) -> u8 {
    let m = message.to_lowercase();
    let l = line.to_lowercase();
    if m.contains("fatal error") || m.contains("parse error") || m.contains("uncaught") || l.contains(":error]") || l.contains("[error]") || l.contains("[crit]") || l.contains("[alert]") || l.contains("[emerg]") {
        1
    } else if m.contains("warning") || l.contains(":warn]") || l.contains("[warn]") {
        2
    } else if m.contains("notice") || m.contains("deprecated") || m.contains("strict standards") {
        3
    } else if m.contains("error") {
        1
    } else {
        2
    }
}

/// `… in /var/www/a.php on line 12` or `… in /var/www/a.php:12`.
fn file_and_line(message: &str) -> Option<(String, usize)> {
    for (i, _) in message.match_indices(" in /") {
        let rest = &message[i + 4..];
        if let Some(on) = rest.find(" on line ") {
            let path = &rest[..on];
            let digits: String = rest[on + 9..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !path.contains(' ')
                && let Ok(n) = digits.parse()
            {
                return Some((path.to_string(), n));
            }
        }
        let token: &str = rest.split([' ', '"', ',', '(', ')']).next().unwrap_or("");
        if let Some((path, n)) = token.rsplit_once(':')
            && let Ok(n) = n.trim_end_matches('.').parse()
        {
            return Some((path.to_string(), n));
        }
    }
    None
}

/// The server path in Nebula's view, given a path as the log names it.
/// FTP and chrooted SFTP see a shorter path (`/public_html/a.php`) than PHP
/// (`/home/user/public_html/a.php`); `known` tells whether a path exists on
/// the server as Nebula sees it (from folder listings). Returns the path and
/// the prefix to strip from further log paths.
pub fn to_site_path(log_path: &str, known: impl Fn(&str) -> bool) -> Option<(String, String)> {
    if known(log_path) {
        return Some((log_path.to_string(), String::new()));
    }
    let mut start = 0;
    while let Some(i) = log_path[start + 1..].find('/') {
        start += 1 + i;
        let candidate = &log_path[start..];
        if known(candidate) {
            return Some((candidate.to_string(), log_path[..start].to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn php_error_log() {
        let e = parse_line("[01-Nov-2026 12:34:56 Europe/Berlin] PHP Warning:  Undefined variable $x in /home/u/public_html/index.php on line 12").unwrap();
        assert_eq!(e.time.as_deref(), Some("01-Nov-2026 12:34:56 Europe/Berlin"));
        assert_eq!(e.severity, 2);
        assert_eq!(e.file, Some(("/home/u/public_html/index.php".into(), 12)));
        assert!(e.message.starts_with("PHP Warning:"));

        let e = parse_line("[01-Nov-2026 12:35:00 UTC] PHP Fatal error:  Uncaught Error: Call to undefined function foo() in /var/www/html/lib/a.php:7").unwrap();
        assert_eq!(e.severity, 1);
        assert_eq!(e.file, Some(("/var/www/html/lib/a.php".into(), 7)));

        assert_eq!(parse_line("Stack trace:"), None);
        assert_eq!(parse_line("#0 /var/www/html/index.php(3): foo()"), None);
        assert_eq!(parse_line("  thrown in /var/www/html/lib/a.php on line 7"), None);
        let e = parse_line("[01-Nov-2026 12:36:00 UTC] PHP Deprecated:  Creation of dynamic property in /var/www/x.php on line 3").unwrap();
        assert_eq!(e.severity, 3);
    }

    #[test]
    fn apache_and_nginx() {
        let e = parse_line("[Sun Nov 01 12:34:56.123456 2026] [php:error] [pid 1234] [client 10.0.0.1:5555] PHP Parse error:  syntax error, unexpected token \"}\" in /var/www/html/x.php on line 3, referer: https://example.com/").unwrap();
        assert_eq!(e.severity, 1);
        assert_eq!(e.file, Some(("/var/www/html/x.php".into(), 3)));
        assert!(e.message.starts_with("PHP Parse error:"), "{}", e.message);
        assert!(!e.message.contains("referer"));

        let e = parse_line("2026/11/01 12:34:56 [error] 123#0: *45 FastCGI sent in stderr: \"PHP message: PHP Warning:  Division by zero in /srv/site/calc.php on line 9\" while reading response header from upstream, client: 1.2.3.4").unwrap();
        assert_eq!(e.time.as_deref(), Some("2026/11/01 12:34:56"));
        assert_eq!(e.message, "PHP Warning:  Division by zero in /srv/site/calc.php on line 9");
        assert_eq!(e.file, Some(("/srv/site/calc.php".into(), 9)));

        let e = parse_line("2026/11/01 12:00:00 [error] 7#7: *1 open() \"/srv/site/missing.png\" failed (2: No such file or directory)").unwrap();
        assert_eq!(e.severity, 1);
        assert_eq!(e.file, None);
    }

    #[test]
    fn site_paths() {
        let known = |p: &str| p == "/public_html/index.php" || p == "/var/www/a.php";
        assert_eq!(to_site_path("/var/www/a.php", known), Some(("/var/www/a.php".into(), String::new())));
        assert_eq!(to_site_path("/home/u/public_html/index.php", known), Some(("/public_html/index.php".into(), "/home/u".into())));
        assert_eq!(to_site_path("/home/u/other.php", known), None);
    }
}
