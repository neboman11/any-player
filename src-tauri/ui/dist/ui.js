// UI controller - manages page navigation and DOM interactions
class UI {
    constructor() {
        this.currentPage = 'now-playing';
        this.currentSource = 'all';
        this.repeatMode = 'off';
        this.shuffle = false;
        this.isPlaying = false;
        this.spotifyAuthWindow = null;
    }

    init() {
        this.setupNavigation();
        this.setupNowPlayingControls();
        this.setupSearchControls();
        this.setupPlaylistTabs();
        this.setupSettingsControls();
        this.checkSpotifyStatus();
        this.updateUI();
    }

    setupNavigation() {
        const navItems = document.querySelectorAll('.nav-item');
        navItems.forEach(item => {
            item.addEventListener('click', (e) => {
                const page = e.currentTarget.dataset.page;
                this.switchPage(page);
            });
        });
    }

    switchPage(pageName) {
        // Hide all pages
        document.querySelectorAll('.page').forEach(page => {
            page.classList.remove('active');
        });

        // Show selected page
        const page = document.getElementById(pageName);
        if (page) {
            page.classList.add('active');
            this.currentPage = pageName;
        }

        // Update nav items
        document.querySelectorAll('.nav-item').forEach(item => {
            item.classList.remove('active');
            if (item.dataset.page === pageName) {
                item.classList.add('active');
            }
        });

        // Load content if needed
        if (pageName === 'playlists') {
            this.loadPlaylists();
        }
    }

    setupNowPlayingControls() {
        const playPauseBtn = document.getElementById('btn-play-pause');
        const nextBtn = document.getElementById('btn-next');
        const previousBtn = document.getElementById('btn-previous');
        const shuffleBtn = document.getElementById('btn-shuffle');
        const repeatBtn = document.getElementById('btn-repeat');
        const volumeSlider = document.getElementById('volume-slider');
        const progressSlider = document.getElementById('progress-slider');

        playPauseBtn?.addEventListener('click', () => this.togglePlayPause());
        nextBtn?.addEventListener('click', () => this.nextTrack());
        previousBtn?.addEventListener('click', () => this.previousTrack());
        shuffleBtn?.addEventListener('click', () => this.toggleShuffle());
        repeatBtn?.addEventListener('click', () => this.nextRepeatMode());
        volumeSlider?.addEventListener('change', (e) => this.setVolume(e.target.value));
        progressSlider?.addEventListener('change', (e) => this.seek(e.target.value));
    }

    setupSearchControls() {
        const searchBtn = document.getElementById('search-btn');
        const searchInput = document.getElementById('search-input');
        const searchTabs = document.querySelectorAll('.search-tabs .tab-btn');
        const searchSourceTabs = document.querySelectorAll('.search-source-tabs .tab-btn');

        searchBtn?.addEventListener('click', () => this.performSearch());
        searchInput?.addEventListener('keypress', (e) => {
            if (e.key === 'Enter') this.performSearch();
        });

        searchTabs.forEach(btn => {
            btn.addEventListener('click', (e) => {
                searchTabs.forEach(b => b.classList.remove('active'));
                e.target.classList.add('active');
            });
        });

        searchSourceTabs.forEach(btn => {
            btn.addEventListener('click', (e) => {
                searchSourceTabs.forEach(b => b.classList.remove('active'));
                e.target.classList.add('active');
                this.currentSource = e.target.dataset.source;
            });
        });
    }

    setupPlaylistTabs() {
        const tabs = document.querySelectorAll('.playlist-tabs .tab-btn');
        tabs.forEach(btn => {
            btn.addEventListener('click', (e) => {
                tabs.forEach(b => b.classList.remove('active'));
                e.target.classList.add('active');
                this.currentSource = e.target.dataset.source;
                this.loadPlaylists();
            });
        });
    }

    setupSettingsControls() {
        const spotifyBtn = document.getElementById('spotify-connect-btn');
        const jellyfinBtn = document.getElementById('jellyfin-connect-btn');

        spotifyBtn?.addEventListener('click', () => this.connectSpotify());
        jellyfinBtn?.addEventListener('click', () => this.connectJellyfin());
    }

    // Playback control methods
    async togglePlayPause() {
        try {
            await tauriAPI.togglePlayPause();
            this.isPlaying = !this.isPlaying;
            this.updatePlayPauseButton();
        } catch (error) {
            console.error('Error toggling play/pause:', error);
        }
    }

    async nextTrack() {
        try {
            await tauriAPI.nextTrack();
            await this.updateUI();
        } catch (error) {
            console.error('Error playing next track:', error);
        }
    }

    async previousTrack() {
        try {
            await tauriAPI.previousTrack();
            await this.updateUI();
        } catch (error) {
            console.error('Error playing previous track:', error);
        }
    }

    async toggleShuffle() {
        try {
            await tauriAPI.toggleShuffle();
            this.shuffle = !this.shuffle;
            this.updateShuffleButton();
        } catch (error) {
            console.error('Error toggling shuffle:', error);
        }
    }

    async nextRepeatMode() {
        const modes = ['off', 'one', 'all'];
        const currentIndex = modes.indexOf(this.repeatMode);
        this.repeatMode = modes[(currentIndex + 1) % modes.length];
        
        try {
            await tauriAPI.setRepeatMode(this.repeatMode);
            this.updateRepeatButton();
        } catch (error) {
            console.error('Error setting repeat mode:', error);
        }
    }

    async setVolume(value) {
        try {
            await tauriAPI.setVolume(parseInt(value));
            document.getElementById('volume-value').textContent = value + '%';
        } catch (error) {
            console.error('Error setting volume:', error);
        }
    }

    async seek(value) {
        try {
            // Convert percentage to milliseconds (assuming 100% = 5 minutes for demo)
            const position = Math.round((value / 100) * 300000);
            await tauriAPI.seek(position);
        } catch (error) {
            console.error('Error seeking:', error);
        }
    }

    // Playlist methods
    async loadPlaylists() {
        try {
            const grid = document.getElementById('playlists-grid');
            grid.innerHTML = '<div class="playlist-card loading">Loading playlists...</div>';
            
            let playlists = [];
            
            // Load Spotify playlists if authenticated
            if (this.currentSource === 'spotify' || this.currentSource === 'all') {
                try {
                    const spotifyPlaylists = await tauriAPI.getSpotifyPlaylists();
                    playlists = playlists.concat(spotifyPlaylists);
                } catch (error) {
                    console.warn('Could not load Spotify playlists:', error);
                }
            }
            
            if (!playlists || playlists.length === 0) {
                grid.innerHTML = '<div class="playlist-card">No playlists found. Connect a service in Settings.</div>';
                return;
            }

            grid.innerHTML = '';
            playlists.forEach(playlist => {
                const card = this.createPlaylistCard(playlist);
                grid.appendChild(card);
            });
        } catch (error) {
            console.error('Error loading playlists:', error);
            document.getElementById('playlists-grid').innerHTML = '<div class="playlist-card">Error loading playlists</div>';
        }
    }

    createPlaylistCard(playlist) {
        const card = document.createElement('div');
        card.className = 'playlist-card';
        card.innerHTML = `
            <h4>${playlist.name}</h4>
            <p>${playlist.owner}</p>
            <p>${playlist.track_count} tracks</p>
            <small>${playlist.source}</small>
        `;
        card.addEventListener('click', () => {
            console.log('Clicked playlist:', playlist.id);
            // TODO: Load playlist details and start playing
        });
        return card;
    }

    // Search methods
    async performSearch() {
        const query = document.getElementById('search-input').value;
        if (!query) return;

        const resultsDiv = document.getElementById('search-results');
        resultsDiv.innerHTML = '<div class="playlist-card loading">Searching...</div>';

        // TODO: Implement actual search using Tauri commands
        // For now, show a placeholder
        setTimeout(() => {
            resultsDiv.innerHTML = '<p>Search functionality coming soon</p>';
        }, 500);
    }

    // Spotify settings methods
    async checkSpotifyStatus() {
        try {
            const isAuthenticated = await tauriAPI.isSpotifyAuthenticated();
            const statusEl = document.getElementById('spotify-status');
            const btnEl = document.getElementById('spotify-connect-btn');
            
            if (isAuthenticated && statusEl && btnEl) {
                statusEl.textContent = '✓ Connected';
                statusEl.className = 'status connected';
                btnEl.textContent = 'Disconnect Spotify';
            }
        } catch (error) {
            console.warn('Could not check Spotify status:', error);
        }
    }

    async connectSpotify() {
        try {
            // Check if we already have credentials in localStorage
            let clientId = localStorage.getItem('spotify_client_id');
            let clientSecret = localStorage.getItem('spotify_client_secret');
            let redirectUri = 'http://localhost:5173/auth/spotify';
            
            if (!clientId || !clientSecret) {
                // Prompt user for credentials
                clientId = prompt('Enter your Spotify Client ID:');
                clientSecret = prompt('Enter your Spotify Client Secret:');
                
                if (!clientId || !clientSecret) {
                    console.log('Spotify credentials cancelled');
                    return;
                }
                
                // Store in localStorage
                localStorage.setItem('spotify_client_id', clientId);
                localStorage.setItem('spotify_client_secret', clientSecret);
            }
            
            const status = document.getElementById('spotify-status');
            if (status) status.textContent = 'Authorizing...';
            
            // Get authorization URL
            const authUrl = await tauriAPI.getSpotifyAuthUrl(clientId, clientSecret, redirectUri);
            
            // Open OAuth URL in system browser or new window
            window.open(authUrl, '_blank', 'width=500,height=600');
            
            // Poll for authentication completion
            this.waitForSpotifyAuth();
            
        } catch (error) {
            console.error('Error connecting to Spotify:', error);
            const status = document.getElementById('spotify-status');
            if (status) status.textContent = '✗ Connection failed';
        }
    }

    async waitForSpotifyAuth() {
        // Wait a bit then check status
        const checkInterval = setInterval(async () => {
            try {
                const isAuthenticated = await tauriAPI.isSpotifyAuthenticated();
                if (isAuthenticated) {
                    clearInterval(checkInterval);
                    const statusEl = document.getElementById('spotify-status');
                    const btnEl = document.getElementById('spotify-connect-btn');
                    if (statusEl) {
                        statusEl.textContent = '✓ Connected';
                        statusEl.className = 'status connected';
                    }
                    if (btnEl) btnEl.textContent = 'Disconnect Spotify';
                }
            } catch (error) {
                // Continue polling
            }
        }, 1000);
        
        // Stop polling after 5 minutes
        setTimeout(() => clearInterval(checkInterval), 300000);
    }

    connectJellyfin() {
        const url = document.getElementById('jellyfin-url').value;
        const apiKey = document.getElementById('jellyfin-api-key').value;
        
        if (!url || !apiKey) {
            alert('Please enter both Jellyfin URL and API key');
            return;
        }

        console.log('Connecting to Jellyfin...', url);
        // TODO: Implement Jellyfin connection
        const status = document.getElementById('jellyfin-status');
        status.textContent = 'Connecting...';
    }

    // UI update methods
    updatePlayPauseButton() {
        const btn = document.getElementById('btn-play-pause');
        if (btn) {
            btn.innerHTML = this.isPlaying ? '<span>⏸</span>' : '<span>▶</span>';
            btn.title = this.isPlaying ? 'Pause' : 'Play';
        }
    }

    updateShuffleButton() {
        const btn = document.getElementById('btn-shuffle');
        if (btn) {
            btn.style.opacity = this.shuffle ? '1' : '0.5';
        }
    }

    updateRepeatButton() {
        const btn = document.getElementById('btn-repeat');
        if (btn) {
            const icons = { 'off': '🔁', 'one': '🔂', 'all': '🔁' };
            btn.innerHTML = `<span>${icons[this.repeatMode]}</span>`;
            btn.style.opacity = this.repeatMode !== 'off' ? '1' : '0.5';
        }
    }

    async updateUI() {
        try {
            const status = await tauriAPI.getPlaybackStatus();
            if (status) {
                this.isPlaying = status.state === 'playing';
                this.shuffle = status.shuffle;
                this.repeatMode = status.repeat_mode;
                
                this.updatePlayPauseButton();
                this.updateShuffleButton();
                this.updateRepeatButton();

                // Update volume
                const volumeSlider = document.getElementById('volume-slider');
                if (volumeSlider) {
                    volumeSlider.value = status.volume;
                    document.getElementById('volume-value').textContent = status.volume + '%';
                }
            }
        } catch (error) {
            console.error('Error updating UI:', error);
        }
    }
}

// Create global UI instance
const ui = new UI();
