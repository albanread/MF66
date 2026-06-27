use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt;
use std::io::{self, Read, Write};

use crate::BulkDescriptor;

pub const MAX_PLUGIN_HOST_FRAME_BYTES: u32 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PluginHostRequest {
    Ping,
    Shutdown,
    JsonParse {
        text: String,
    },
    JsonQuery {
        value: i64,
        path: String,
    },
    JsonFree {
        value: i64,
    },
    JsonExists {
        value: i64,
        path: String,
    },
    JsonType {
        value: i64,
    },
    JsonLen {
        value: i64,
    },
    JsonRows {
        value: i64,
    },
    JsonCols {
        value: i64,
    },
    JsonColName {
        value: i64,
        col: i64,
    },
    JsonRowsTableData {
        value: i64,
        row_start: i64,
        row_count: i64,
        col_start: i64,
        col_count: i64,
    },
    JsonInt {
        value: i64,
    },
    JsonText {
        value: i64,
    },
    JsonStringify {
        value: i64,
    },
    JsonIsNull {
        value: i64,
    },
    JsonLastError,
    JsonMarkTaint,
    SqliteOpen {
        path: String,
    },
    SqliteClose {
        conn: i64,
    },
    SqliteExec {
        conn: i64,
        sql: String,
    },
    SqliteQuery {
        conn: i64,
        sql: String,
    },
    SqliteRows {
        rows: i64,
    },
    SqliteCols {
        rows: i64,
    },
    SqliteRowsTableData {
        rows: i64,
        row_start: i64,
        row_count: i64,
        col_start: i64,
        col_count: i64,
    },
    SqliteInt {
        rows: i64,
        row: i64,
        col: i64,
    },
    SqliteText {
        rows: i64,
        row: i64,
        col: i64,
    },
    SqliteFree {
        rows: i64,
    },
    SqliteLastError,
    SqliteOpenFile {
        path: String,
    },
    SqliteOpenFileDyn {
        path: String,
    },
    SqliteOpenMemory,
    SqliteOpenMemoryDyn,
    SqlitePrepare {
        conn: i64,
        sql: String,
    },
    SqliteBindInt {
        stmt: i64,
        value: i64,
    },
    SqliteBindText {
        stmt: i64,
        text: String,
    },
    SqliteBindNull {
        stmt: i64,
    },
    SqliteStmtQuery {
        stmt: i64,
    },
    SqliteStmtExec {
        stmt: i64,
    },
    SqliteStmtReset {
        stmt: i64,
    },
    SqliteFinalize {
        stmt: i64,
    },
    SqliteIsNull {
        rows: i64,
        row: i64,
        col: i64,
    },
    CredentialNull,
    CredentialIsValid {
        credential: i64,
    },
    CredentialAwsDefaultSession,
    CredentialFetchAwsSecret {
        session: i64,
        secret_id: String,
    },
    CredentialOpenNamed {
        name: String,
    },
    CredentialMarkAccess {
        name: String,
    },
    CredentialRegisterJson {
        secret_json: String,
    },
    CredentialClose {
        credential: i64,
    },
    CredentialEngine {
        credential: i64,
    },
    CredentialField {
        credential: i64,
        key: String,
    },
    CredentialHandleValue {
        credential: i64,
    },
    CredentialLastError,
    MysqlDriverReady,
    MysqlNullConnection,
    MysqlConnectionIsValid {
        conn: i64,
    },
    MysqlOpenCredential {
        credential: i64,
    },
    MysqlOpenNamed {
        name: String,
    },
    MysqlClose {
        conn: i64,
    },
    MysqlExec {
        conn: i64,
        sql: String,
    },
    MysqlQuery {
        conn: i64,
        sql: String,
    },
    MysqlNullStatement,
    MysqlStatementIsValid {
        stmt: i64,
    },
    MysqlPrepare {
        conn: i64,
        sql: String,
    },
    MysqlBindInt {
        stmt: i64,
        value: i64,
    },
    MysqlBindText {
        stmt: i64,
        text: String,
    },
    MysqlBindNull {
        stmt: i64,
    },
    MysqlReset {
        stmt: i64,
    },
    MysqlRunQuery {
        stmt: i64,
    },
    MysqlRunExec {
        stmt: i64,
    },
    MysqlFinalize {
        stmt: i64,
    },
    MysqlNullRows,
    MysqlRowsIsValid {
        rows: i64,
    },
    MysqlRows {
        rows: i64,
    },
    MysqlCols {
        rows: i64,
    },
    MysqlRowsTableData {
        rows: i64,
        row_start: i64,
        row_count: i64,
        col_start: i64,
        col_count: i64,
    },
    MysqlInt {
        rows: i64,
        row: i64,
        col: i64,
    },
    MysqlText {
        rows: i64,
        row: i64,
        col: i64,
    },
    MysqlIsNull {
        rows: i64,
        row: i64,
        col: i64,
    },
    MysqlFree {
        rows: i64,
    },
    MysqlLastError,
    WritePreparedBulk {
        handle: i64,
        descriptor: BulkDescriptor,
    },
    DropPreparedBulk {
        handle: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PluginHostResponse {
    Pong,
    Ok,
    I64 {
        value: i64,
    },
    String {
        value: String,
        tainted: bool,
    },
    PreparedBulk {
        handle: i64,
        byte_len: u64,
        format: i64,
        tainted: bool,
    },
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub enum PluginHostFrameError {
    Io(io::Error),
    FrameTooLarge(u32),
    Codec(String),
}

impl fmt::Display for PluginHostFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::FrameTooLarge(len) => write!(f, "plugin-host frame too large: {len} bytes"),
            Self::Codec(err) => write!(f, "plugin-host codec error: {err}"),
        }
    }
}

impl std::error::Error for PluginHostFrameError {}

impl From<io::Error> for PluginHostFrameError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn write_plugin_frame<W, T>(writer: &mut W, message: &T) -> Result<(), PluginHostFrameError>
where
    W: Write,
    T: Serialize,
{
    let bytes = postcard::to_allocvec(message)
        .map_err(|err| PluginHostFrameError::Codec(err.to_string()))?;
    if bytes.len() > MAX_PLUGIN_HOST_FRAME_BYTES as usize {
        return Err(PluginHostFrameError::FrameTooLarge(bytes.len() as u32));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn read_plugin_frame<R, T>(reader: &mut R) -> Result<Option<T>, PluginHostFrameError>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err.into()),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_PLUGIN_HOST_FRAME_BYTES {
        return Err(PluginHostFrameError::FrameTooLarge(len));
    }
    let mut bytes = vec![0u8; len as usize];
    reader.read_exact(&mut bytes)?;
    let message =
        postcard::from_bytes(&bytes).map_err(|err| PluginHostFrameError::Codec(err.to_string()))?;
    Ok(Some(message))
}
