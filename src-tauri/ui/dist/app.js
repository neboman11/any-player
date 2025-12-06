// Main application initialization
document.addEventListener('DOMContentLoaded', async () => {
    console.log('Initializing Any Player Desktop UI...');
    
    // Wait for Tauri API to be ready before initializing UI
    await tauriAPI.init();
    
    // Initialize UI
    ui.init();

    // Start periodic UI updates
    setInterval(() => {
        ui.updateUI();
    }, 500);

    console.log('Any Player Desktop UI ready!');
});
