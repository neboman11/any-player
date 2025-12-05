/// Main entry point for Any Player CLI
use clap::Parser;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(
    name = "Any Player",
    version = "0.1.0",
    about = "Multi-source music client supporting Spotify and Jellyfin",
    long_about = None
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Configuration directory
    #[arg(short, long)]
    config: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Run in daemon mode
    #[arg(short, long)]
    daemon: bool,
}

#[derive(Parser, Debug)]
enum Command {
    /// Start the interactive UI
    Tui,

    /// List playlists from a source
    List {
        /// Source provider (spotify, jellyfin, or both)
        #[arg(short, long, default_value = "both")]
        source: String,
    },

    /// Search for playlists or tracks
    Search {
        /// Search query
        query: String,

        /// Source provider (spotify, jellyfin, or both)
        #[arg(short, long, default_value = "both")]
        source: String,

        /// Search for playlists instead of tracks
        #[arg(short, long)]
        playlists: bool,
    },

    /// Play a playlist or track
    Play {
        /// Playlist or track ID
        id: String,

        /// Source provider (spotify or jellyfin)
        #[arg(short, long)]
        source: String,
    },

    /// Create a new playlist
    CreatePlaylist {
        /// Playlist name
        name: String,

        /// Source provider (spotify or jellyfin)
        #[arg(short, long)]
        source: String,

        /// Playlist description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Add track to playlist
    AddTrack {
        /// Playlist ID
        playlist_id: String,

        /// Track ID
        track_id: String,

        /// Source provider for the track
        #[arg(short, long)]
        source: String,
    },

    /// Authenticate with a provider
    Auth {
        /// Provider to authenticate with (spotify or jellyfin)
        provider: String,
    },

    /// Show current playback status
    Status,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize tracing
    init_logging(&args.log_level)?;

    info!("Starting Any Player v0.1.0");

    // Load configuration
    let _config = any_player::Config::load()?;
    info!("Configuration loaded");

    // Handle commands
    match args.command {
        Some(Command::Tui) => {
            info!("Starting TUI mode");
            run_tui().await?;
        }
        Some(Command::List { source }) => {
            handle_list_command(&source).await?;
        }
        Some(Command::Search {
            query,
            source,
            playlists,
        }) => {
            handle_search_command(&query, &source, playlists).await?;
        }
        Some(Command::Play { id, source }) => {
            handle_play_command(&id, &source).await?;
        }
        Some(Command::CreatePlaylist {
            name,
            source,
            description,
        }) => {
            handle_create_playlist_command(&name, &source, description).await?;
        }
        Some(Command::AddTrack {
            playlist_id,
            track_id,
            source,
        }) => {
            handle_add_track_command(&playlist_id, &track_id, &source).await?;
        }
        Some(Command::Auth { provider }) => {
            handle_auth_command(&provider).await?;
        }
        Some(Command::Status) => {
            handle_status_command().await?;
        }
        None => {
            // Default: start TUI
            run_tui().await?;
        }
    }

    Ok(())
}

fn init_logging(level: &str) -> Result<(), Box<dyn std::error::Error>> {
    let level_filter: tracing_subscriber::filter::LevelFilter = level.parse()?;
    tracing_subscriber::fmt()
        .with_max_level(level_filter)
        .init();
    Ok(())
}

async fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Implement TUI using ratatui
    println!("TUI mode not yet implemented");
    Ok(())
}

async fn handle_list_command(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Listing playlists from: {}", source);
    // TODO: Implement list command
    Ok(())
}

async fn handle_search_command(
    query: &str,
    source: &str,
    playlists: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let search_type = if playlists { "playlists" } else { "tracks" };
    println!("Searching for {} in {} from {}", query, search_type, source);
    // TODO: Implement search command
    Ok(())
}

async fn handle_play_command(id: &str, source: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Playing {} from {}", id, source);
    // TODO: Implement play command
    Ok(())
}

async fn handle_create_playlist_command(
    name: &str,
    source: &str,
    description: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating playlist '{}' in {}", name, source);
    if let Some(desc) = description {
        println!("Description: {}", desc);
    }
    // TODO: Implement create playlist command
    Ok(())
}

async fn handle_add_track_command(
    playlist_id: &str,
    track_id: &str,
    source: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Adding track {} to playlist {} in {}",
        track_id, playlist_id, source
    );
    // TODO: Implement add track command
    Ok(())
}

async fn handle_auth_command(provider: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Authenticating with {}", provider);
    // TODO: Implement authentication
    Ok(())
}

async fn handle_status_command() -> Result<(), Box<dyn std::error::Error>> {
    println!("Current playback status:");
    // TODO: Implement status command
    Ok(())
}
