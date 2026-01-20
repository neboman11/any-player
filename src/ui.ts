/**
 * UI controller - manages page navigation and DOM interactions
 */

import { tauriAPI } from "./api";
import type {
  OAuthCallbackData,
  Playlist,
  RepeatMode,
  SearchResult,
  SearchType,
  TauriSource,
  Track,
} from "./types";

export class UI {
  private currentPage: string = "now-playing";
  private currentSource: TauriSource = "all";
  private repeatMode: RepeatMode = "off";
  private shuffle: boolean = false;
  private isPlaying: boolean = false;

  init(): void {
    this.setupNavigation();
    this.setupNowPlayingControls();
    this.setupSearchControls();
    this.setupPlaylistTabs();
    this.setupSettingsControls();
    void this.checkSpotifyStatus();
    void this.checkJellyfinStatus();
    void this.updateUI();

    // Listen for OAuth callback messages from auth window
    window.addEventListener("message", (event: MessageEvent) => {
      const data = event.data as OAuthCallbackData;
      if (data && data.type === "spotify-auth") {
        if (data.code) {
          console.log("Received auth code from popup");
          void this.completeSpotifyAuth(data.code);
        } else if (data.error) {
          console.error("Auth error:", data.error);
          const status = document.getElementById("spotify-status");
          if (status) status.textContent = `✗ ${data.error}`;
        }
      }
    });
  }

  private setupNavigation(): void {
    const navItems = document.querySelectorAll<HTMLButtonElement>(".nav-item");
    navItems.forEach((item) => {
      item.addEventListener("click", (e) => {
        const target = e.currentTarget as HTMLButtonElement;
        const page = target.dataset.page;
        if (page) {
          this.switchPage(page);
        }
      });
    });
  }

  private switchPage(pageName: string): void {
    // Hide all pages
    document.querySelectorAll<HTMLElement>(".page").forEach((page) => {
      page.classList.remove("active");
    });

    // Show selected page
    const page = document.getElementById(pageName);
    if (page) {
      page.classList.add("active");
      this.currentPage = pageName;
    }

    // Update nav items
    document
      .querySelectorAll<HTMLButtonElement>(".nav-item")
      .forEach((item) => {
        item.classList.remove("active");
        if (item.dataset.page === pageName) {
          item.classList.add("active");
        }
      });

    // Load content if needed
    if (pageName === "playlists") {
      void this.loadPlaylists();
    }
  }

  private setupNowPlayingControls(): void {
    const playPauseBtn = document.getElementById("btn-play-pause");
    const nextBtn = document.getElementById("btn-next");
    const previousBtn = document.getElementById("btn-previous");
    const shuffleBtn = document.getElementById("btn-shuffle");
    const repeatBtn = document.getElementById("btn-repeat");
    const volumeSlider = document.getElementById("volume-slider");
    const progressSlider = document.getElementById("progress-slider");

    playPauseBtn?.addEventListener("click", () => void this.togglePlayPause());
    nextBtn?.addEventListener("click", () => void this.nextTrack());
    previousBtn?.addEventListener("click", () => void this.previousTrack());
    shuffleBtn?.addEventListener("click", () => void this.toggleShuffle());
    repeatBtn?.addEventListener("click", () => void this.nextRepeatMode());
    volumeSlider?.addEventListener("change", (e) => {
      const value = (e.target as HTMLInputElement).value;
      void this.setVolume(value);
    });
    progressSlider?.addEventListener("change", (e) => {
      const value = (e.target as HTMLInputElement).value;
      void this.seek(value);
    });
  }

  private setupSearchControls(): void {
    const searchBtn = document.getElementById("search-btn");
    const searchInput = document.getElementById("search-input");
    const searchTabs = document.querySelectorAll<HTMLButtonElement>(
      ".search-tabs .tab-btn",
    );
    const searchSourceTabs = document.querySelectorAll<HTMLButtonElement>(
      ".search-source-tabs .tab-btn",
    );

    searchBtn?.addEventListener("click", () => void this.performSearch());
    searchInput?.addEventListener("keypress", (e) => {
      if ((e as KeyboardEvent).key === "Enter") void this.performSearch();
    });

    searchTabs.forEach((btn) => {
      btn.addEventListener("click", (e) => {
        searchTabs.forEach((b) => b.classList.remove("active"));
        (e.target as HTMLButtonElement).classList.add("active");
      });
    });

    searchSourceTabs.forEach((btn) => {
      btn.addEventListener("click", (e) => {
        searchSourceTabs.forEach((b) => b.classList.remove("active"));
        (e.target as HTMLButtonElement).classList.add("active");
        const source = (e.target as HTMLButtonElement).dataset
          .source as TauriSource;
        this.currentSource = source;
      });
    });
  }

  private setupPlaylistTabs(): void {
    const tabs = document.querySelectorAll<HTMLButtonElement>(
      ".playlist-tabs .tab-btn",
    );
    tabs.forEach((btn) => {
      btn.addEventListener("click", (e) => {
        tabs.forEach((b) => b.classList.remove("active"));
        (e.target as HTMLButtonElement).classList.add("active");
        const source = (e.target as HTMLButtonElement).dataset
          .source as TauriSource;
        this.currentSource = source;
        void this.loadPlaylists();
      });
    });
  }

  private setupSettingsControls(): void {
    const spotifyBtn = document.getElementById("spotify-connect-btn");
    const jellyfinBtn = document.getElementById("jellyfin-connect-btn");

    spotifyBtn?.addEventListener("click", () => void this.connectSpotify());
    jellyfinBtn?.addEventListener("click", () => this.connectJellyfin());
  }

  // Playback control methods
  private async togglePlayPause(): Promise<void> {
    try {
      await tauriAPI.togglePlayPause();
      this.isPlaying = !this.isPlaying;
      this.updatePlayPauseButton();
    } catch (error) {
      console.error("Error toggling play/pause:", error);
    }
  }

  private async nextTrack(): Promise<void> {
    try {
      await tauriAPI.nextTrack();
      await this.updateUI();
    } catch (error) {
      console.error("Error playing next track:", error);
    }
  }

  private async previousTrack(): Promise<void> {
    try {
      await tauriAPI.previousTrack();
      await this.updateUI();
    } catch (error) {
      console.error("Error playing previous track:", error);
    }
  }

  private async toggleShuffle(): Promise<void> {
    try {
      await tauriAPI.toggleShuffle();
      this.shuffle = !this.shuffle;
      this.updateShuffleButton();
    } catch (error) {
      console.error("Error toggling shuffle:", error);
    }
  }

  private async nextRepeatMode(): Promise<void> {
    const modes: RepeatMode[] = ["off", "one", "all"];
    const currentIndex = modes.indexOf(this.repeatMode);
    this.repeatMode = modes[(currentIndex + 1) % modes.length];

    try {
      await tauriAPI.setRepeatMode(this.repeatMode);
      this.updateRepeatButton();
    } catch (error) {
      console.error("Error setting repeat mode:", error);
    }
  }

  private async setVolume(value: string): Promise<void> {
    try {
      await tauriAPI.setVolume(parseInt(value, 10));
      const volumeValue = document.getElementById("volume-value");
      if (volumeValue) {
        volumeValue.textContent = `${value}%`;
      }
    } catch (error) {
      console.error("Error setting volume:", error);
    }
  }

  private async seek(value: string): Promise<void> {
    try {
      // Convert percentage to milliseconds (assuming 100% = 5 minutes for demo)
      const position = Math.round((parseInt(value, 10) / 100) * 300000);
      await tauriAPI.seek(position);
    } catch (error) {
      console.error("Error seeking:", error);
    }
  }

  // Playlist methods
  private async loadPlaylists(): Promise<void> {
    try {
      const grid = document.getElementById("playlists-grid");
      if (!grid) return;

      grid.innerHTML =
        '<div class="playlist-card loading">Loading playlists...</div>';

      let playlists: Playlist[] = [];

      // Load Spotify playlists if authenticated
      if (this.currentSource === "spotify" || this.currentSource === "all") {
        try {
          const spotifyPlaylists = await tauriAPI.getSpotifyPlaylists();
          playlists = playlists.concat(spotifyPlaylists);
        } catch (error) {
          console.warn("Could not load Spotify playlists:", error);
        }
      }

      // Load Jellyfin playlists if authenticated
      if (this.currentSource === "jellyfin" || this.currentSource === "all") {
        try {
          const jellyfinPlaylists = await tauriAPI.getJellyfinPlaylists();
          playlists = playlists.concat(jellyfinPlaylists);
        } catch (error) {
          console.warn("Could not load Jellyfin playlists:", error);
        }
      }

      if (!playlists || playlists.length === 0) {
        grid.innerHTML =
          '<div class="playlist-card">No playlists found. Connect a service in Settings.</div>';
        return;
      }

      grid.innerHTML = "";
      playlists.forEach((playlist) => {
        const card = this.createPlaylistCard(playlist);
        grid.appendChild(card);
      });
    } catch (error) {
      console.error("Error loading playlists:", error);
      const grid = document.getElementById("playlists-grid");
      if (grid) {
        grid.innerHTML =
          '<div class="playlist-card">Error loading playlists</div>';
      }
    }
  }

  private createPlaylistCard(playlist: Playlist): HTMLDivElement {
    const card = document.createElement("div");
    card.className = "playlist-card";
    card.innerHTML = `
            <h4>${playlist.name}</h4>
            <p>${playlist.owner}</p>
            <p>${playlist.track_count} tracks</p>
            <small>${playlist.source}</small>
        `;
    card.addEventListener("click", () => {
      console.log("Clicked playlist:", playlist.id);
      // TODO: Load playlist details and start playing
    });
    return card;
  }

  // Search methods
  private async performSearch(): Promise<void> {
    const searchInput = document.getElementById("search-input");
    if (!(searchInput instanceof HTMLInputElement)) return;

    const query = searchInput.value;
    if (!query) return;

    const resultsDiv = document.getElementById("search-results");
    if (!resultsDiv) return;

    resultsDiv.innerHTML =
      '<div class="playlist-card loading">Searching...</div>';

    try {
      // Get the active search tab (tracks or playlists)
      const activeTab = document.querySelector<HTMLButtonElement>(
        ".search-tabs .tab-btn.active",
      );
      const searchType =
        (activeTab?.dataset.type as SearchType | undefined) ?? "tracks";

      // Get the active source tab
      const activeSourceTab = document.querySelector<HTMLButtonElement>(
        ".search-source-tabs .tab-btn.active",
      );
      const source =
        (activeSourceTab?.dataset.source as TauriSource | undefined) ?? "all";

      let results: SearchResult[] = [];

      if (searchType === "tracks") {
        // Search for tracks
        if (source === "spotify" || source === "all") {
          try {
            // Note: We'd need to implement search_spotify_tracks in the backend
            // For now, Spotify playlists are loaded via get_spotify_playlists
          } catch {
            console.warn("Spotify track search not yet implemented");
          }
        }

        if (source === "jellyfin" || source === "all") {
          try {
            const jellyfinTracks = await tauriAPI.searchJellyfinTracks(query);
            results = results.concat(
              jellyfinTracks.map((track: Track) => ({
                id: track.id,
                name: track.title,
                artist: track.artist,
                type: "track" as const,
                source: track.source,
              })),
            );
          } catch (error) {
            console.warn("Could not search Jellyfin tracks:", error);
          }
        }
      } else {
        // Search for playlists
        if (source === "jellyfin" || source === "all") {
          try {
            const jellyfinPlaylists =
              await tauriAPI.searchJellyfinPlaylists(query);
            results = results.concat(
              jellyfinPlaylists.map((p: Playlist) => ({
                id: p.id,
                name: p.name,
                owner: p.owner,
                type: "playlist" as const,
                source: p.source,
              })),
            );
          } catch (error) {
            console.warn("Could not search Jellyfin playlists:", error);
          }
        }
      }

      if (!results || results.length === 0) {
        resultsDiv.innerHTML = "<p>No results found</p>";
        return;
      }

      resultsDiv.innerHTML = "";
      results.forEach((result) => {
        const resultCard = document.createElement("div");
        resultCard.className = "search-result-card playlist-card";

        if (result.type === "track") {
          resultCard.innerHTML = `
                        <h4>${result.name}</h4>
                        <p>${result.artist}</p>
                        <small>${result.source}</small>
                    `;
        } else {
          resultCard.innerHTML = `
                        <h4>${result.name}</h4>
                        <p>by ${result.owner}</p>
                        <small>${result.source}</small>
                    `;
        }

        resultCard.addEventListener("click", () => {
          console.log("Clicked search result:", result.id);
          // TODO: Load and play the selected item
        });

        resultsDiv.appendChild(resultCard);
      });
    } catch (error) {
      console.error("Error performing search:", error);
      resultsDiv.innerHTML = "<p>Error during search</p>";
    }
  }

  // Spotify settings methods
  private async checkSpotifyStatus(): Promise<void> {
    try {
      const isAuthenticated = await tauriAPI.isSpotifyAuthenticated();
      const statusEl = document.getElementById("spotify-status");
      const btnEl = document.getElementById("spotify-connect-btn");

      if (isAuthenticated && statusEl && btnEl) {
        statusEl.textContent = "✓ Connected";
        statusEl.className = "status connected";
        btnEl.textContent = "Disconnect Spotify";
      }
    } catch (error) {
      console.warn("Could not check Spotify status:", error);
    }
  }

  private async connectSpotify(): Promise<void> {
    try {
      const status = document.getElementById("spotify-status");
      if (status) status.textContent = "Opening Spotify login...";

      // Get authorization URL (no credentials needed!)
      const authUrl = await tauriAPI.getSpotifyAuthUrl();
      console.log("Auth URL:", authUrl);

      // Try to open in default browser using Tauri shell
      if (window.__TAURI__?.shell) {
        try {
          await window.__TAURI__.shell.open(authUrl);
          if (status) status.textContent = "Waiting for authentication...";
        } catch (e) {
          console.error("Failed to open browser:", e);
          this.showAuthFallback(authUrl);
        }
      } else {
        // Fallback: show the link to the user
        this.showAuthFallback(authUrl);
      }

      // Start waiting for the OAuth callback
      void this.waitForSpotifyAuth();
    } catch (error) {
      console.error("Error connecting to Spotify:", error);
      const status = document.getElementById("spotify-status");
      if (status) status.textContent = "✗ Connection failed";
    }
  }

  private showAuthFallback(authUrl: string): void {
    // Show a modal with the auth link
    const fallbackDiv = document.createElement("div");
    fallbackDiv.id = "auth-fallback";
    fallbackDiv.style.cssText = `
            position: fixed;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: rgba(0, 0, 0, 0.8);
            display: flex;
            align-items: center;
            justify-content: center;
            z-index: 10000;
        `;

    const modalDiv = document.createElement("div");
    modalDiv.style.cssText = `
            background: white;
            padding: 30px;
            border-radius: 10px;
            max-width: 500px;
            text-align: center;
            box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
        `;

    modalDiv.innerHTML = `
            <h2>Complete Spotify Login</h2>
            <p style="margin: 20px 0; color: #666;">Click the button below to log in to Spotify, or copy the link:</p>
            <div style="margin: 20px 0;">
                <a href="${authUrl}" target="_blank" style="
                    display: inline-block;
                    background: #1DB954;
                    color: white;
                    padding: 12px 30px;
                    border-radius: 25px;
                    text-decoration: none;
                    font-weight: bold;
                    margin-bottom: 15px;
                ">Open Spotify Login</a>
            </div>
            <p style="font-size: 12px; color: #999; margin: 15px 0;">Or copy this link:</p>
            <div style="
                background: #f5f5f5;
                padding: 10px;
                border-radius: 5px;
                margin: 10px 0;
                word-break: break-all;
                font-family: monospace;
                font-size: 12px;
            ">
                <input type="text" value="${authUrl}" readonly style="
                    width: 100%;
                    border: none;
                    background: transparent;
                    font-family: monospace;
                    font-size: 12px;
                    padding: 5px;
                " id="auth-url-input">
            </div>
            <button id="copy-link-btn" style="
                background: #ddd;
                border: none;
                padding: 8px 16px;
                border-radius: 5px;
                cursor: pointer;
                font-size: 12px;
            ">Copy Link</button>
            <p style="margin-top: 20px; color: #666; font-size: 14px;">
                After logging in, this window will automatically close.
            </p>
            <button id="close-modal-btn" style="
                margin-top: 15px;
                background: #f0f0f0;
                border: none;
                padding: 10px 20px;
                border-radius: 5px;
                cursor: pointer;
            ">Close</button>
        `;

    fallbackDiv.appendChild(modalDiv);
    document.body.appendChild(fallbackDiv);

    // Add event listeners for buttons
    const copyBtn = document.getElementById("copy-link-btn");
    if (copyBtn) {
      copyBtn.addEventListener("click", (e) => {
        const input = document.getElementById("auth-url-input");
        if (input instanceof HTMLInputElement) {
          input.select();
          document.execCommand("copy");
          const originalText = (e.target as HTMLButtonElement).textContent;
          (e.target as HTMLButtonElement).textContent = "Copied!";
          setTimeout(() => {
            (e.target as HTMLButtonElement).textContent = originalText;
          }, 2000);
        }
      });
    }

    const closeBtn = document.getElementById("close-modal-btn");
    if (closeBtn) {
      closeBtn.addEventListener("click", () => {
        const fallback = document.getElementById("auth-fallback");
        if (fallback) fallback.remove();
      });
    }
  }

  private async waitForSpotifyAuth(): Promise<void> {
    // Poll for authentication completion
    let pollCount = 0;
    const maxPolls = 600; // 10 minutes at 1 second intervals

    const checkInterval = setInterval(async () => {
      pollCount++;
      try {
        // First, check if there's a pending OAuth code to process
        const codeProcessed = await tauriAPI.checkOAuthCode();
        if (codeProcessed) {
          console.log("OAuth code processed successfully");
        }

        // Then check if authenticated
        const isAuthenticated = await tauriAPI.isSpotifyAuthenticated();
        console.log(`Auth check ${pollCount}: ${isAuthenticated}`);

        if (isAuthenticated) {
          clearInterval(checkInterval);

          // Remove fallback modal if it exists
          const fallback = document.getElementById("auth-fallback");
          if (fallback) fallback.remove();

          const statusEl = document.getElementById("spotify-status");
          const btnEl = document.getElementById("spotify-connect-btn");
          if (statusEl) {
            statusEl.textContent = "✓ Connected";
            statusEl.className = "status connected";
          }
          if (btnEl) btnEl.textContent = "Disconnect Spotify";

          console.log("Spotify authentication successful!");
        }
      } catch (error) {
        console.error("Error checking auth status:", error);
      }

      // Stop polling after max attempts
      if (pollCount >= maxPolls) {
        clearInterval(checkInterval);
        console.log("Auth polling timeout after 10 minutes");
        const statusEl = document.getElementById("spotify-status");
        if (
          statusEl &&
          statusEl.textContent === "Waiting for authentication..."
        ) {
          statusEl.textContent = "⏱ Auth timeout - please try again";
        }
      }
    }, 1000);
  }

  private async completeSpotifyAuth(code: string): Promise<void> {
    try {
      const status = document.getElementById("spotify-status");
      if (status) status.textContent = "Completing authentication...";

      // Send the auth code to the backend
      await tauriAPI.authenticateSpotify(code);

      // Update UI
      const statusEl = document.getElementById("spotify-status");
      const btnEl = document.getElementById("spotify-connect-btn");
      if (statusEl) {
        statusEl.textContent = "✓ Connected";
        statusEl.className = "status connected";
      }
      if (btnEl) btnEl.textContent = "Disconnect Spotify";

      console.log("Spotify authentication successful!");
      // Try to initialize the librespot session on the backend using the
      // provider-managed token. This ensures the session is actually
      // prepared for premium playback when the UI shows connected state.
      try {
        await tauriAPI.initializeSpotifySessionFromProvider();
        // Verify session readiness and reflect in the UI
        const ready = await tauriAPI.isSpotifySessionReady();
        const statusAfter = document.getElementById("spotify-status");
        if (ready && statusAfter) {
          statusAfter.textContent = "✓ Connected (session ready)";
          statusAfter.className = "status connected";
        } else if (statusAfter) {
          statusAfter.textContent = "⚠ Connected (session not ready)";
          statusAfter.className = "status warn";
        }
      } catch (err) {
        console.warn("Failed to initialize Spotify session from UI:", err);
        const statusAfter = document.getElementById("spotify-status");
        if (statusAfter) {
          statusAfter.textContent = "⚠ Connected (session init failed)";
          statusAfter.className = "status warn";
        }
      }
    } catch (error) {
      console.error("Error completing Spotify auth:", error);
      const status = document.getElementById("spotify-status");
      if (status) status.textContent = "✗ Auth failed";
    }
  }

  private connectJellyfin(): void {
    const urlInput = document.getElementById("jellyfin-url");
    const apiKeyInput = document.getElementById("jellyfin-api-key");

    if (
      !(urlInput instanceof HTMLInputElement) ||
      !(apiKeyInput instanceof HTMLInputElement)
    ) {
      return;
    }

    const url = urlInput.value;
    const apiKey = apiKeyInput.value;

    if (!url || !apiKey) {
      alert("Please enter both Jellyfin URL and API key");
      return;
    }

    void this.performJellyfinConnection(url, apiKey);
  }

  private async performJellyfinConnection(
    url: string,
    apiKey: string,
  ): Promise<void> {
    try {
      const status = document.getElementById("jellyfin-status");
      if (status) status.textContent = "Connecting...";

      // Attempt to authenticate with Jellyfin
      await tauriAPI.authenticateJellyfin(url, apiKey);

      // Check if authentication was successful
      const isAuthenticated = await tauriAPI.isJellyfinAuthenticated();

      if (isAuthenticated) {
        if (status) {
          status.textContent = "✓ Connected";
          status.className = "status connected";
        }
        const btnEl = document.getElementById("jellyfin-connect-btn");
        if (btnEl) btnEl.textContent = "Disconnect Jellyfin";

        console.log("Jellyfin authentication successful!");

        // Reload playlists if they're currently displayed
        if (this.currentPage === "playlists") {
          void this.loadPlaylists();
        }
      } else {
        if (status) status.textContent = "✗ Connection failed";
      }
    } catch (error) {
      console.error("Error connecting to Jellyfin:", error);
      const status = document.getElementById("jellyfin-status");
      if (status) {
        status.textContent = `✗ Connection failed: ${
          error instanceof Error ? error.message : "Unknown error"
        }`;
      }
    }
  }

  private async checkJellyfinStatus(): Promise<void> {
    try {
      const isAuthenticated = await tauriAPI.isJellyfinAuthenticated();
      const statusEl = document.getElementById("jellyfin-status");
      const btnEl = document.getElementById("jellyfin-connect-btn");

      if (isAuthenticated && statusEl && btnEl) {
        statusEl.textContent = "✓ Connected";
        statusEl.className = "status connected";
        btnEl.textContent = "Disconnect Jellyfin";
      }
    } catch (error) {
      console.warn("Could not check Jellyfin status:", error);
    }
  }

  // UI update methods
  private updatePlayPauseButton(): void {
    const btn = document.getElementById("btn-play-pause");
    if (btn) {
      btn.innerHTML = this.isPlaying ? "<span>⏸</span>" : "<span>▶</span>";
      btn.title = this.isPlaying ? "Pause" : "Play";
    }
  }

  private updateShuffleButton(): void {
    const btn = document.getElementById("btn-shuffle");
    if (btn) {
      btn.style.opacity = this.shuffle ? "1" : "0.5";
    }
  }

  private updateRepeatButton(): void {
    const btn = document.getElementById("btn-repeat");
    if (btn) {
      const icons: Record<RepeatMode, string> = {
        off: "🔁",
        one: "🔂",
        all: "🔁",
      };
      btn.innerHTML = `<span>${icons[this.repeatMode]}</span>`;
      btn.style.opacity = this.repeatMode !== "off" ? "1" : "0.5";
    }
  }

  async updateUI(): Promise<void> {
    try {
      const status = await tauriAPI.getPlaybackStatus();
      if (status) {
        this.isPlaying = status.state === "playing";
        this.shuffle = status.shuffle;
        this.repeatMode = status.repeat_mode;

        this.updatePlayPauseButton();
        this.updateShuffleButton();
        this.updateRepeatButton();

        // Update volume
        const volumeSlider = document.getElementById("volume-slider");
        if (volumeSlider instanceof HTMLInputElement) {
          volumeSlider.value = String(status.volume);
          const volumeValue = document.getElementById("volume-value");
          if (volumeValue) {
            volumeValue.textContent = `${status.volume}%`;
          }
        }
      }
    } catch (error) {
      console.error("Error updating UI:", error);
    }
  }
}

// Create and export global UI instance
export const ui = new UI();
