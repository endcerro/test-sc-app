//! Resolving a git ref of scunpacked-data to a commit, and downloading files at that commit.

use serde::Deserialize;

use crate::DataError;
use crate::http::HttpClient;

/// A resolved commit of the data repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub sha: String,
    /// Game version, taken from the commit message (`4.10.2-LIVE.12881860`).
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    sha: String,
    commit: CommitDetail,
}

#[derive(Debug, Deserialize)]
struct CommitDetail {
    message: String,
}

/// Resolves `git_ref` (branch, tag or sha) of `repo` (`owner/name`).
///
/// Tries the GitHub REST API first (gives the version). Falls back to git smart HTTP
/// (`info/refs`), which has no rate limit but no commit message.
pub fn resolve_revision(
    http: &dyn HttpClient,
    repo: &str,
    git_ref: &str,
) -> Result<Revision, DataError> {
    if is_sha(git_ref) {
        return Ok(Revision {
            sha: git_ref.to_owned(),
            version: None,
        });
    }
    match resolve_via_api(http, repo, git_ref) {
        Ok(rev) => Ok(rev),
        Err(api_err) => {
            tracing::debug!("GitHub API lookup failed ({api_err}), trying git smart HTTP");
            let url = format!("https://github.com/{repo}.git/info/refs?service=git-upload-pack");
            let body = http.get(&url, None)?;
            let sha =
                parse_info_refs(&body, git_ref).ok_or_else(|| DataError::InvalidResponse {
                    url,
                    message: format!("ref {git_ref:?} not found"),
                })?;
            Ok(Revision {
                sha,
                version: version_from_ref(git_ref),
            })
        }
    }
}

fn resolve_via_api(
    http: &dyn HttpClient,
    repo: &str,
    git_ref: &str,
) -> Result<Revision, DataError> {
    let url = format!("https://api.github.com/repos/{repo}/commits/{git_ref}");
    let body = http.get(&url, Some("application/vnd.github+json"))?;
    let resp: CommitResponse = serde_json::from_slice(&body).map_err(|source| DataError::Json {
        what: url.clone(),
        source,
    })?;
    if !is_sha(&resp.sha) {
        return Err(DataError::InvalidResponse {
            url,
            message: format!("not a commit sha: {:?}", resp.sha),
        });
    }
    let version = version_from_message(&resp.commit.message).or_else(|| version_from_ref(git_ref));
    Ok(Revision {
        sha: resp.sha,
        version,
    })
}

/// URL of a file at a given commit.
pub fn raw_url(repo: &str, sha: &str, file: &str) -> String {
    format!("https://raw.githubusercontent.com/{repo}/{sha}/{file}")
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// scunpacked-data commits are titled with the game version.
fn version_from_message(message: &str) -> Option<String> {
    let first = message.lines().next()?.trim();
    looks_like_version(first).then(|| first.to_owned())
}

/// Version branches are named after the game version (`3.24.2-LIVE.9327108`).
fn version_from_ref(git_ref: &str) -> Option<String> {
    looks_like_version(git_ref).then(|| git_ref.to_owned())
}

fn looks_like_version(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.contains('.')
        && !s.contains(char::is_whitespace)
}

/// Parses a git smart-HTTP `info/refs` advertisement (pkt-line format) and returns the sha
/// for `git_ref` (`HEAD`, a branch or a tag name).
pub(crate) fn parse_info_refs(body: &[u8], git_ref: &str) -> Option<String> {
    let wanted = [
        git_ref.to_owned(),
        format!("refs/heads/{git_ref}"),
        format!("refs/tags/{git_ref}^{{}}"),
        format!("refs/tags/{git_ref}"),
    ];
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut rest = body;
    while rest.len() >= 4 {
        let len = usize::from_str_radix(std::str::from_utf8(&rest[..4]).ok()?, 16).ok()?;
        if len == 0 {
            rest = &rest[4..];
            continue;
        }
        if len < 4 || len > rest.len() {
            return None;
        }
        let line = std::str::from_utf8(&rest[4..len]).ok()?;
        rest = &rest[len..];
        let line = line.split('\0').next().unwrap_or_default().trim_end();
        if let Some((sha, name)) = line.split_once(' ')
            && is_sha(sha)
            && let Some(rank) = wanted.iter().position(|w| w == name)
        {
            found.push((rank, sha.to_owned()));
        }
    }
    found.sort();
    found.into_iter().next().map(|(_, sha)| sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(s: &str) -> String {
        format!("{:04x}{s}", s.len() + 4)
    }

    const A: &str = "5ee0a3dd20ba0e8dfd13512c57a2309231c8b8f7";
    const B: &str = "8702de155cb2bb152d7e13ad27045802730f4fff";

    #[test]
    fn parses_info_refs() {
        let body = format!(
            "{}0000{}{}{}0000",
            pkt("# service=git-upload-pack\n"),
            pkt(&format!(
                "{A} HEAD\0multi_ack symref=HEAD:refs/heads/master\n"
            )),
            pkt(&format!("{B} refs/heads/3.12.1l\n")),
            pkt(&format!("{A} refs/heads/master\n")),
        );
        assert_eq!(
            parse_info_refs(body.as_bytes(), "master").as_deref(),
            Some(A)
        );
        assert_eq!(parse_info_refs(body.as_bytes(), "HEAD").as_deref(), Some(A));
        assert_eq!(
            parse_info_refs(body.as_bytes(), "3.12.1l").as_deref(),
            Some(B)
        );
        assert_eq!(parse_info_refs(body.as_bytes(), "nope"), None);
        assert_eq!(parse_info_refs(b"garbage", "master"), None);
    }

    #[test]
    fn versions() {
        assert_eq!(
            version_from_message("4.10.2-LIVE.12881860\n\nbody").as_deref(),
            Some("4.10.2-LIVE.12881860")
        );
        assert_eq!(version_from_message("Update README"), None);
        assert_eq!(version_from_ref("master"), None);
        assert_eq!(
            version_from_ref("3.24.2-LIVE.9327108").as_deref(),
            Some("3.24.2-LIVE.9327108")
        );
    }
}
