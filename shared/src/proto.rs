//! Framed request/response protocol spoken over the agent's stdin/stdout.
//!
//! Every frame is `u32 header_len | header JSON | u32 payload_len | payload`.
//! The payload carries raw file contents so they never go through JSON.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::fsops::{DirEntry, SearchMatch, SearchQuery, Stat};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Op {
    Hello,
    Stat {
        path: String,
    },
    ReadDir {
        path: String,
    },
    ReadFile {
        path: String,
    },
    /// Contents are sent in the payload.
    WriteFile {
        path: String,
    },
    CreateDir {
        path: String,
    },
    Remove {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    ListFiles {
        root: String,
        max: usize,
    },
    Search {
        root: String,
        query: SearchQuery,
    },
    Watch {
        root: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Request {
    pub id: u64,
    pub op: Op,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Reply {
    Ok,
    Hello {
        version: String,
        arch: String,
    },
    Stat(Stat),
    Entries(Vec<DirEntry>),
    /// Contents are in the payload.
    Data,
    Files(Vec<String>),
    Matches {
        matches: Vec<SearchMatch>,
        truncated: bool,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Message {
    Response { id: u64, result: Result<Reply, String> },
    FsChanged { paths: Vec<String> },
}

const MAX_FRAME: usize = 512 * 1024 * 1024;

pub fn write_frame<W: Write>(writer: &mut W, header: &impl Serialize, payload: &[u8]) -> io::Result<()> {
    let header = serde_json::to_vec(header).map_err(io::Error::other)?;
    writer.write_all(&(header.len() as u32).to_le_bytes())?;
    writer.write_all(&header)?;
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<(T, Vec<u8>)> {
    let header = read_chunk(reader)?;
    let payload = read_chunk(reader)?;
    let header = serde_json::from_slice(&header).map_err(io::Error::other)?;
    Ok((header, payload))
}

fn read_chunk<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other("frame too large"));
    }
    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// Executes a request against the local filesystem. Used by the agent and by
/// the editor itself for local projects.
pub fn execute(op: &Op, payload: &[u8]) -> Result<(Reply, Vec<u8>), String> {
    use crate::fsops::*;
    let err = |e: std::io::Error| e.to_string();
    Ok(match op {
        Op::Hello => (Reply::Hello { version: crate::VERSION.to_string(), arch: std::env::consts::ARCH.to_string() }, Vec::new()),
        Op::Stat { path } => (Reply::Stat(stat(path).map_err(err)?), Vec::new()),
        Op::ReadDir { path } => (Reply::Entries(read_dir(path).map_err(err)?), Vec::new()),
        Op::ReadFile { path } => (Reply::Data, std::fs::read(path).map_err(err)?),
        Op::WriteFile { path } => {
            write_atomic(path, payload).map_err(err)?;
            (Reply::Ok, Vec::new())
        }
        Op::CreateDir { path } => {
            std::fs::create_dir_all(path).map_err(err)?;
            (Reply::Ok, Vec::new())
        }
        Op::Remove { path } => {
            remove(path).map_err(err)?;
            (Reply::Ok, Vec::new())
        }
        Op::Rename { from, to } => {
            std::fs::rename(from, to).map_err(err)?;
            (Reply::Ok, Vec::new())
        }
        Op::ListFiles { root, max } => (Reply::Files(list_files(root, *max)), Vec::new()),
        Op::Search { root, query } => {
            let (matches, truncated) = search(root, query)?;
            (Reply::Matches { matches, truncated }, Vec::new())
        }
        Op::Watch { .. } => return Err("watch is handled by the agent loop".into()),
    })
}
