//! Launch the local Spotify client when a user requests playback.
use ap_playback::spotify::{SpotifyError, SpotifyLauncher};
use async_trait::async_trait;
use tokio::process::Command;

#[derive(Debug, Default, Clone, Copy)]
pub struct DesktopSpotifyLauncher;

#[async_trait]
impl SpotifyLauncher for DesktopSpotifyLauncher {
    fn hostname(&self) -> String {
        gethostname::gethostname().to_string_lossy().into_owned()
    }
    async fn launch(&self) -> Result<(), SpotifyError> {
        #[cfg(target_os = "windows")]
        let candidates: &[(&str, &[&str])] = &[("cmd", &["/C", "start", "", "spotify:"])];
        #[cfg(target_os = "macos")]
        let candidates: &[(&str, &[&str])] = &[("open", &["-a", "Spotify"])];
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let candidates: &[(&str, &[&str])] = &[
            ("spotify", &[]),
            ("flatpak", &["run", "com.spotify.Client"]),
            ("xdg-open", &["spotify:"]),
        ];
        for (program, args) in candidates {
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            if *program == "flatpak" {
                let installed = Command::new("flatpak")
                    .args(["info", "com.spotify.Client"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .is_ok_and(|status| status.success());
                if !installed {
                    continue;
                }
            }
            if Command::new(program).args(*args).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(SpotifyError::NotInstalled)
    }
}
