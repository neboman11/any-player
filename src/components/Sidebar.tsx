import type { Page } from "../types";

interface SidebarProps {
  currentPage: Page;
  setCurrentPage: (page: Page) => void;
}

const pages = [
  { id: "now-playing" as const, icon: "▶", label: "Now Playing" },
  { id: "playlists" as const, icon: "📋", label: "Playlists" },
  { id: "search" as const, icon: "🔍", label: "Search" },
  { id: "settings" as const, icon: "⚙️", label: "Settings" },
];

export function Sidebar({ currentPage, setCurrentPage }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="logo">
        <h1>🎵 Any Player</h1>
      </div>
      <nav className="nav-menu">
        {pages.map((page) => (
          <button
            key={page.id}
            className={`nav-item ${currentPage === page.id ? "active" : ""}`}
            onClick={() => setCurrentPage(page.id)}
          >
            <span className="icon">{page.icon}</span>
            <span>{page.label}</span>
          </button>
        ))}
      </nav>
    </aside>
  );
}
