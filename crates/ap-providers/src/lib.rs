//! Provider clients for Any Player (design §7): Jellyfin, Plex and the Spotify Web API, the
//! Spotify PKCE sign-in and token provider, the Spotify Connect player API, and the shared HTTP
//! client and retry policy.
//!
//! Credentials arrive as values; token changes leave through [`spotify::TokenSink`]. Nothing here
//! touches storage. Stored track and artwork URLs never carry credentials: they are added as
//! headers at request time ([`ProviderClient::stream_request`], [`ProviderClient::image_request`]).

pub mod error;
pub mod http;
pub mod jellyfin;
mod json_ext;
pub mod plex;
pub mod provider;
pub mod spotify;

pub use error::ProviderError;
pub use http::{
    RetryPolicy, build_client, client_builder, execute_retrying, execute_retrying_with,
};
pub use jellyfin::{JellyfinClient, JellyfinConfig, JellyfinUser};
pub use plex::{PlexClient, PlexConfig};
pub use provider::{MediaRequest, ProviderClient, Validation, strip_url_tokens};
pub use spotify::{SpotifyClient, SpotifyPlayerApi, SpotifyTokenProvider, TokenSink};
