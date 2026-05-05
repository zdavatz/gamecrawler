//! Minimal FlareSolverr client. Used to fetch BGG browse pages, which sit
//! behind Cloudflare's managed challenge and would otherwise reject our
//! plain reqwest. The user is expected to have FlareSolverr running on
//! :8191 (default port).
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_ENDPOINT: &str = "http://localhost:8191/v1";

pub struct FlareSolverr {
    endpoint: String,
    http: Client,
}

impl FlareSolverr {
    pub fn new() -> Result<Self> {
        Self::with_endpoint(
            std::env::var("FLARESOLVERR_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_ENDPOINT.into()),
        )
    }

    pub fn with_endpoint(endpoint: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            endpoint: endpoint.into(),
            http,
        })
    }

    pub async fn get(&self, url: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Req<'a> {
            cmd: &'a str,
            url: &'a str,
            #[serde(rename = "maxTimeout")]
            max_timeout: u32,
        }
        #[derive(Deserialize)]
        struct Resp {
            status: String,
            message: Option<String>,
            solution: Option<Solution>,
        }
        #[derive(Deserialize)]
        struct Solution {
            response: String,
            status: u16,
        }

        let req = Req {
            cmd: "request.get",
            url,
            max_timeout: 60_000,
        };
        let resp: Resp = self
            .http
            .post(&self.endpoint)
            .json(&req)
            .send()
            .await
            .with_context(|| format!("POST {} (is FlareSolverr running on :8191?)", self.endpoint))?
            .error_for_status()?
            .json()
            .await?;
        if resp.status != "ok" {
            return Err(anyhow!(
                "flaresolverr: {}",
                resp.message.unwrap_or_else(|| "unknown".into())
            ));
        }
        let sol = resp
            .solution
            .ok_or_else(|| anyhow!("flaresolverr returned no solution"))?;
        if sol.status >= 400 {
            return Err(anyhow!("upstream HTTP {} for {}", sol.status, url));
        }
        Ok(sol.response)
    }
}
