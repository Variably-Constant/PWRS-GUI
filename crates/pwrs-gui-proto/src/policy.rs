//! A page's content-security policy: the policy a page of the caller's own
//! carries unless it is given one, the sources `-Connect` adds to its
//! `connect-src`, and a policy given whole, which must fit on the one line
//! an HTTP header holds. The session builds a page's policy with these and
//! the renderer checks what it is given with them.

/// The policy a page of the caller's own carries unless it is given one:
/// its own origin, inline script and style, eval, and data and blob
/// addresses, so nothing remote loads and the page reaches only its own
/// origin.
pub const DEFAULT: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'";

/// The sources a `connect-src` made for `-Connect` starts with: those
/// [`DEFAULT`]'s `default-src` lets a page reach, so the page still fetches
/// its own files and data and blob addresses.
const OWN_CONNECT: &str = "'self' data: blob:";

/// [`DEFAULT`] with a `connect-src` of the page's own sources and `sources`,
/// each one [`connect_source`] accepts: the default policy, changed only in
/// the addresses the page may fetch from and open sockets and event streams
/// to. Refused when `sources` is empty or holds a source that is not one.
pub fn with_connect(sources: &[String]) -> Result<String, String> {
    if sources.is_empty() {
        return Err(String::from("no source was given to add to the page's connect-src"));
    }
    for s in sources {
        connect_source(s)?;
    }
    Ok(format!("{DEFAULT}; connect-src {OWN_CONNECT} {}", sources.join(" ")))
}

/// Whether `source` is one a `connect-src` may be given beside the page's
/// own: a scheme, as `https:` or `wss:`, or a host with an optional scheme,
/// port and path, as `https://api.example.com`, `ws://localhost:8080`,
/// `https://*.example.com` or `*`, written as a content-security policy
/// writes them. A keyword, a nonce, a hash, a quote, a comma, a semicolon,
/// white space and a host in brackets are refused, since each either names
/// no address or would change more of the policy than its `connect-src`.
pub fn connect_source(source: &str) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("{source:?} is not a source a page may connect to: {why}"));
    if source.is_empty() {
        return refuse("it is empty");
    }
    if let Some(c) = source.chars().find(|c| c.is_whitespace() || c.is_control() || matches!(c, ';' | ',' | '\'' | '"')) {
        return refuse(&format!("it holds {c:?}, which ends a source or begins a keyword"));
    }
    if !source.is_ascii() {
        return refuse("a source is written in ASCII, its host in its punycode form");
    }
    if let Some(scheme) = source.strip_suffix(':')
        && !scheme.contains(':')
        && !scheme.contains('/')
    {
        return match is_scheme(scheme) {
            true => Ok(()),
            false => refuse("a scheme is a letter followed by letters, digits, plus signs, hyphens or dots"),
        };
    }
    let rest = match source.split_once("://") {
        Some((scheme, rest)) if is_scheme(scheme) => rest,
        Some(_) => return refuse("a scheme is a letter followed by letters, digits, plus signs, hyphens or dots"),
        None => source,
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if host.starts_with('[') {
        return refuse("a host in brackets is not one a content-security policy can name; use its name, or 127.0.0.1 or localhost");
    }
    is_host(host).or_else(|why| refuse(&why))?;
    if let Some(port) = port
        && port != "*"
    {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return refuse("a port is a number up to 65535, or *");
        }
        if let Err(too_large) = port.parse::<u16>() {
            return refuse(&format!("a port is a number up to 65535, or *, and {port} is not: {too_large}"));
        }
    }
    if path.contains("//") {
        return refuse("a path holds no empty part");
    }
    if let Some(c) = path.chars().find(|c| !(c.is_ascii_alphanumeric() || "-._~%!$&()*+=:@/".contains(*c))) {
        return refuse(&format!("a path cannot hold {c:?}"));
    }
    Ok(())
}

/// Whether `scheme` is a URL scheme: a letter, then letters, digits, plus
/// signs, hyphens and dots.
fn is_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Whether `host` is a policy's host: `*`, or names of letters, digits and
/// hyphens joined by dots, the first of them `*` for any name under the
/// rest, a dot allowed at the end; or why not.
fn is_host(host: &str) -> Result<(), String> {
    if host == "*" {
        return Ok(());
    }
    let named = host.strip_prefix("*.").unwrap_or(host);
    let named = named.strip_suffix('.').unwrap_or(named);
    if named.is_empty() {
        return Err(String::from("it names no host"));
    }
    match named.split('.').all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
        true => Ok(()),
        false => Err(String::from("a host is names of letters, digits and hyphens joined by dots, the first of them * for any name under the rest")),
    }
}

/// Whether `policy` is one a page's files can carry as given: some text on
/// one line, in ASCII. What it allows is the caller's to say; the engine
/// reads it as it reads any page's policy.
pub fn whole_policy(policy: &str) -> Result<(), String> {
    if policy.trim().is_empty() {
        return Err(String::from("the policy is empty"));
    }
    if let Some(c) = policy.chars().find(|c| (c.is_control() && *c != '\t') || !c.is_ascii()) {
        return Err(format!("the policy holds {c:?}; a policy is one line of ASCII, as an HTTP header carries it"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_adds_its_sources_to_a_connect_src_beside_the_page_s_own_and_changes_nothing_else() {
        let sources = vec![String::from("https://api.example.com"), String::from("ws://localhost:8080")];
        let policy = with_connect(&sources).expect("two sources");
        assert_eq!(policy, "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'; connect-src 'self' data: blob: https://api.example.com ws://localhost:8080");
        assert!(policy.starts_with(DEFAULT), "the default policy stays as it is");
        assert!(with_connect(&[]).is_err(), "no source to add");
    }

    #[test]
    fn a_source_is_a_scheme_or_a_host_with_its_scheme_port_and_path() {
        for good in ["https:", "wss:", "web+app:", "https://api.example.com", "ws://localhost:8080", "https://*.example.com", "https://*", "*", "localhost", "127.0.0.1:9000", "https://example.com:*", "https://example.com/api/v1/", "http://example.com.", "https://a-b.example.com/x%20y"] {
            if let Err(why) = connect_source(good) {
                panic!("{good} is refused: {why}");
            }
        }
    }

    #[test]
    fn a_source_that_names_no_address_or_would_change_more_of_the_policy_is_refused() {
        for (bad, why) in [
            ("", "empty"),
            ("'self'", "keyword"),
            ("'unsafe-inline'", "keyword"),
            ("https://a.example script-src *", "' '"),
            ("https://a.example;script-src", "';'"),
            ("https://a.example,https://b.example", "','"),
            ("https://[::1]:8080", "brackets"),
            ("1http://a.example", "scheme"),
            ("https://a..example", "host"),
            ("https://a.*.example", "host"),
            ("https://", "no host"),
            ("https://a.example:99999", "65535"),
            ("https://a.example:8o", "port"),
            ("https://a.example//x", "empty part"),
            ("https://a.example/a\\b", "'\\\\'"),
            ("https://bücher.example", "ASCII"),
            ("https://a.example\n", "'\\n'"),
        ] {
            let refused = connect_source(bad).expect_err(bad);
            assert!(refused.contains(why), "{bad:?}: {refused}");
        }
    }

    #[test]
    fn a_whole_policy_is_one_line_of_ascii() {
        for good in ["default-src 'none'; script-src 'self'", "default-src 'self';\tconnect-src *"] {
            if let Err(why) = whole_policy(good) {
                panic!("{good:?} is refused: {why}");
            }
        }
        for bad in ["", "   ", "default-src 'self'\r\nX-Other: 1", "default-src https://bücher.example"] {
            assert!(whole_policy(bad).is_err(), "{bad:?} is no policy a header carries");
        }
    }
}
