//! Update notice: the newest release on GitHub, compared with this version.

/// Where releases are published.
pub const REPO: &str = "Euphonique/nebula";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Version without a leading "v".
    pub version: String,
    /// Page of the release.
    pub url: String,
}

/// Whether `latest` is newer than `current` ("0.2.0" > "0.1.9", "v1.0" > "0.9.3").
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
    };
    let (a, b) = (parts(latest), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The newest release, if there is one; `Ok(None)` when the repository has none
/// (or can't be seen, e.g. while it is private).
pub fn latest() -> Result<Option<Release>, String> {
    latest_of(REPO)
}

fn latest_of(repo: &str) -> Result<Option<Release>, String> {
    let http: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent(concat!("Nebula/", env!("CARGO_PKG_VERSION")))
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut response = http.get(&url).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        404 => return Ok(None),
        200 => {}
        code => return Err(t!("GitHub antwortet mit {code}", code = code)),
    }
    let json: serde_json::Value = serde_json::from_reader(response.body_mut().as_reader()).map_err(|e| e.to_string())?;
    Ok(parse(&json))
}

fn parse(json: &serde_json::Value) -> Option<Release> {
    let tag = json["tag_name"].as_str()?;
    Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: json["html_url"].as_str().unwrap_or("").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against GitHub (network): the private repo shows nothing, a public one its release.
    #[test]
    #[ignore]
    fn github_live() {
        println!("nebula: {:?}", latest_of(REPO));
        println!("ripgrep: {:?}", latest_of("BurntSushi/ripgrep"));
    }

    #[test]
    fn versions_and_release() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.3"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        let json = serde_json::json!({"tag_name": "v0.3.1", "html_url": "https://github.com/x/y/releases/tag/v0.3.1"});
        assert_eq!(parse(&json), Some(Release { version: "0.3.1".into(), url: "https://github.com/x/y/releases/tag/v0.3.1".into() }));
    }
}
