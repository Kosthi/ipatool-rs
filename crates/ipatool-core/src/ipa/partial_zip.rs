use std::io::{Read, Seek, SeekFrom};

use crate::error::ClientError;

pub struct HttpReader {
    client: reqwest::Client,
    url: String,
    size: u64,
    pos: u64,
    runtime: tokio::runtime::Handle,
}

impl HttpReader {
    pub async fn new(client: &reqwest::Client, url: &str) -> Result<Self, ClientError> {
        let resp = client.get(url).header("Range", "bytes=0-0").send().await?;

        let size = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| {
                ClientError::UnexpectedResponse("missing Content-Range for size".into())
            })?;

        Ok(Self {
            client: client.clone(),
            url: url.to_string(),
            size,
            pos: 0,
            runtime: tokio::runtime::Handle::current(),
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.size {
            return Ok(0);
        }

        let end = std::cmp::min(self.pos + buf.len() as u64 - 1, self.size - 1);
        let range = format!("bytes={}-{}", self.pos, end);

        let resp = self
            .runtime
            .block_on(async {
                self.client
                    .get(&self.url)
                    .header("Range", &range)
                    .send()
                    .await?
                    .bytes()
                    .await
            })
            .map_err(std::io::Error::other)?;

        let n = resp.len();
        buf[..n].copy_from_slice(&resp);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.pos = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::End(n) => (self.size as i64 + n) as u64,
            SeekFrom::Current(n) => (self.pos as i64 + n) as u64,
        };
        Ok(self.pos)
    }
}

pub async fn read_remote_info_plist(
    client: &reqwest::Client,
    url: &str,
) -> Result<plist::Dictionary, ClientError> {
    let reader = HttpReader::new(client, url).await?;
    let size = reader.size();

    let archive = zip::ZipArchive::new(reader)
        .map_err(|e| ClientError::UnexpectedResponse(format!("zip: {e}")))?;

    let info_plist_name = archive
        .file_names()
        .find(|name| {
            name.starts_with("Payload/")
                && name.ends_with(".app/Info.plist")
                && !name.contains("Watch/")
                && name.matches('/').count() == 2
        })
        .map(String::from)
        .ok_or_else(|| ClientError::UnexpectedResponse("no Info.plist found".into()))?;

    let mut archive = zip::ZipArchive::new(HttpReader {
        client: client.clone(),
        url: url.to_string(),
        size,
        pos: 0,
        runtime: tokio::runtime::Handle::current(),
    })
    .map_err(|e| ClientError::UnexpectedResponse(format!("zip reopen: {e}")))?;

    let mut entry = archive
        .by_name(&info_plist_name)
        .map_err(|e| ClientError::UnexpectedResponse(format!("zip entry: {e}")))?;

    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| ClientError::UnexpectedResponse(format!("zip read: {e}")))?;

    plist::from_bytes(&buf).map_err(ClientError::PlistDe)
}
