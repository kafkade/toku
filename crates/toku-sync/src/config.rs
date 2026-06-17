use clap::Parser;

/// Toku sync relay server — stores and relays sync operations between devices.
#[derive(Debug, Parser)]
#[command(name = "toku-sync", version)]
pub struct Config {
    /// Port to listen on
    #[arg(long, default_value = "8080", env = "TOKU_SYNC_PORT")]
    pub port: u16,

    /// Address to bind to
    #[arg(long, default_value = "0.0.0.0", env = "TOKU_SYNC_BIND")]
    pub bind: String,

    /// Directory for server data (SQLite database)
    #[arg(long, default_value = "./toku-sync-data", env = "TOKU_SYNC_DATA_DIR")]
    pub data_dir: String,
}

impl Config {
    pub fn db_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.data_dir).join("sync.db")
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}
