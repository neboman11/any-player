// Main application initialization
document.addEventListener('DOMContentLoaded', () => {
    console.log('Initializing Any Player Desktop UI...');
    
    // Initialize UI
    ui.init();

    // Start periodic UI updates
    setInterval(() => {
        ui.updateUI();
    }, 500);

    console.log('Any Player Desktop UI ready!');
});
