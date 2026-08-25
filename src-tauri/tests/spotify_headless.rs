use any_player_lib::config::Config;
use any_player_lib::providers::spotify::SpotifyProvider;
use any_player_spotify_engine::LibrespotPlayer;

#[tokio::test]
#[ignore = "requires real Spotify token in keyring and an active playback device"]
async fn headless_spotify_playback() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let tokens = Config::load_tokens().expect("load_tokens");
    let token = tokens.spotify_token.expect("no spotify token in keyring");

    let mut provider = SpotifyProvider::with_default_oauth();
    provider.set_token(token).await.expect("set_token/refresh");
    let access_token = provider
        .get_access_token()
        .await
        .expect("no access token after refresh");

    let player = LibrespotPlayer::new();
    player
        .connect(&access_token)
        .await
        .expect("connect failed");

    player
        .start_queue(&["198zDKzyktXRG1PGpidY9h".to_string()], 0)
        .await
        .expect("start_queue failed");

    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    let snapshot = player.snapshot().await;
    println!("SNAPSHOT: {:?}", snapshot);
    assert!(snapshot.progress_ms > 0, "playback never advanced");
}
