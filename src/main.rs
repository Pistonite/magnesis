use std::{io::Cursor, path::PathBuf, process::ExitCode, sync::Arc, time::Instant};

use clap::Parser;
use error_stack::{report, Result, ResultExt};
use reqwest::{header::{HeaderMap, HeaderName, HeaderValue}, Client};
use tokio::{fs, process::Command, spawn, task::JoinSet};

/// Pull artifacts from GitHub Actions
#[derive(Debug, clap::Parser)]
struct Cli {
    /// Path to the output directory.
    #[clap(short, long, default_value = "dist")]
    output: String,

    /// Repo to use, default to deriving from the origin remote
    #[clap(long)]
    repo: Option<String>,

    /// Revision (commit/branch) to use
    #[clap(long, default_value = "HEAD")]
    rev: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let start = Instant::now();
    if let Err(err) =  main_internal(cli).await {
        eprintln!("---");
        eprintln!("error: {:?}", err);
        return ExitCode::FAILURE;
    }
    println!("---");
    println!("done in {:.02}s", start.elapsed().as_secs_f64());
    ExitCode::SUCCESS
}

async fn main_internal(cli: Cli) -> Result<(), Error> {
    let token = get_token()?;
    let Cli { output, repo, rev } = cli;
    let output = spawn(create_output(output));
    let repo = spawn(async move {
        match repo {
            Some(repo) => Ok(repo),
            None => get_repo().await,
        }
    });
    let rev = spawn(get_rev(rev));

    let mut headers = HeaderMap::new();
    let token = HeaderValue::from_str(&format!("Bearer {}", token)).change_context(Error::InvalidToken)?;
    headers.insert("Authorization", token);
    headers.insert("User-Agent", HeaderValue::from_name(HeaderName::from_static("reqwest")));
    let client = Client::builder().default_headers(headers).build()
        .change_context(Error::RequestClient)?;

    let repo = repo.await.change_context(Error::Repo)? .attach_printable("please specify the repo with --repo or see GitHub README for more details")? ;
    println!("getting artifacts from repo `{}`", repo);

    let artifacts = get_artifacts(&client, &repo).await.change_context(Error::GetArtifacts)?;
    let rev = rev.await.change_context(Error::Rev)?
    .attach_printable("please specify the revision with --rev or see GitHub README for more details")?
    ;
    println!("finding artifacts for revision `{}`", rev);
    let artifacts = artifacts.into_filtered_by_rev(&rev)?;
    println!("found {} artifacts", artifacts.len());

    let output = output.await.change_context(Error::CreateOutput)??;
    println!("created output at `{}`", output.display());

    let client = Arc::new(client);
    let mut handles = JoinSet::new();

    for artifact in artifacts {
        println!("downloading `{}`", artifact.name);
        let client = Arc::clone(&client);
        let out_dir = output.clone();
        handles.spawn(async move {
            artifact.download(&client, out_dir).await
        });
    }

    while let Some(result) = handles.join_next().await {
        result.change_context(Error::DownloadArtifact)??;
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("failed to create output directory")]
    CreateOutput,
    #[error("failed to get repo")]
    Repo,
    #[error("failed to get rev")]
    Rev,
    #[error("no token provided")]
    NoToken,
    #[error("invalid token")]
    InvalidToken,
    #[error("failed to get artifacts")]
    GetArtifacts,
    #[error("failed to download artifact")]
    DownloadArtifact,
    #[error("failed to parse response")]
    Parse,
    #[error("failed to run command")]
    Command,
    #[error("failed to build request client")]
    RequestClient,
    #[error("request failed")]
    Request,
    #[error("artifact expired")]
    Expired,
    #[error("failed to extract artifact")]
    Extract,
}

async fn create_output(output: String) -> Result<PathBuf, Error> {
    let path = PathBuf::from(&output);
    if path.exists() {
        println!("removing existing output at `{}`", output);
        fs::remove_dir_all(&path).await.
            change_context(Error::CreateOutput)?;
    }
    fs::create_dir_all(&path).await
        .change_context(Error::CreateOutput)?;
    Ok(path)
}

async fn get_rev(rev: String) -> Result<String, Error> {
    if rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()){
        return Ok(rev);
    }


    let output = Command::new("git")
        .args(&["rev-parse", &rev])
        .output()
        .await
        .change_context(Error::Command)?;
    if !output.status.success() {
        return Err(report!(Error::Command))
        .attach_printable(format!("status: {}", output.status));
    }
    let decoded = std::str::from_utf8(&output.stdout)
        .change_context(Error::Command)?;
    Ok(decoded.trim().to_string())
}

async fn get_repo() -> Result<String, Error> {
    let output = Command::new("git")
        .args(&["remote", "get-url", "origin"])
        .output()
        .await
        .change_context(Error::Command)?;
    if !output.status.success() {
        return Err(report!(Error::Command))
        .attach_printable(format!("status: {}", output.status));
    }
    let decoded = std::str::from_utf8(&output.stdout)
        .change_context(Error::Command)?.trim();

    let repo = if let Some(repo) = decoded.strip_prefix("http://github.com/") {
        repo
    } else if let Some(repo) = decoded.strip_prefix("https://github.com/") {
        repo
    } else if let Some(repo) = decoded.strip_prefix("git@github.com:") {
        repo
    } else {
        return Err(report!(Error::Repo))
        .attach_printable(format!("failed to get repo from: {}", decoded));
    };

    Ok(repo.strip_suffix(".git").unwrap_or(repo).to_string())
}

fn get_token() -> Result<String, Error> {
    let message = "please specify the PAT in the GITHUB_TOKEN environment variable";
    let token = std::env::var("GITHUB_TOKEN")
        .change_context(Error::NoToken)
        .attach_printable(message)?;
    if token.is_empty() {
        return Err(report!(Error::NoToken)).attach_printable(message);
    }
    Ok(token)
}

async fn get_artifacts(client: &Client, repo: &str) -> Result<Artifacts, Error> {
    let response = client.get(&format!("https://api.github.com/repos/{}/actions/artifacts", repo))
        .send()
        .await
        .change_context(Error::Request)?
        .error_for_status()
        .change_context(Error::Request)?;
    let bytes = response.bytes().await.change_context(Error::Request)?;
    let value = serde_json::from_slice(&bytes).change_context(Error::Parse)?;

    Ok(value)
}

#[derive(Debug, serde::Deserialize)]
struct Artifacts {
    artifacts: Vec<Artifact>,
}

impl Artifacts {
    pub fn into_filtered_by_rev(self, rev: &str) -> Result<Vec<Artifact>, Error> {
        let artifacts = self.artifacts.into_iter()
            .filter(|artifact| artifact.workflow_run.head_sha == rev)
            .collect::<Vec<_>>();

        if artifacts.is_empty() {
            return Err(report!(Error::GetArtifacts))
            .attach_printable("no artifacts found for the specified revision");
        }

        Ok(artifacts)
    }
}

#[derive(Debug, serde::Deserialize)]
struct Artifact {
    name: String,
    archive_download_url: String,
    workflow_run: WorkflowRun,
}

impl Artifact {
    pub async fn download(&self, client: &Client, out_dir: PathBuf) -> Result<(), Error> {
        self.download_internal(client, out_dir).await
            .change_context(Error::DownloadArtifact)
            .attach_printable_lazy(|| format!("artifact: {}", self.name))
            .attach_printable_lazy(|| format!("url: {}", self.archive_download_url))
    }

    async fn download_internal(
        &self, client: &Client, mut out_dir: PathBuf) -> Result<(), Error> {
        let response = client
            .get(&self.archive_download_url)
            .send()
            .await
            .change_context(Error::Request)
        ?;

        if response.status() == 410 {
            return Err(report!(Error::Expired));
        } else if response.status() != 200 {
            return Err(report!(Error::Request)).attach_printable(
                format!("status: {}", response.status())
            );
        }

        let bytes = response.bytes().await.change_context(Error::Request)?;

        out_dir.push(&self.name);
        println!("extracting `{}`", self.name);
        zip_extract::extract(Cursor::new(bytes), &out_dir, false).change_context(Error::Extract)?;
        println!("downloaded `{}`", self.name);

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct WorkflowRun {
    head_sha: String,
}
