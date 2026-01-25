interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function SearchBar({
  value,
  onChange,
  placeholder = "Search tracks...",
}: SearchBarProps) {
  return (
    <div
      className="search-container"
      style={{ padding: "10px", marginBottom: "10px" }}
    >
      <input
        type="text"
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        style={{
          width: "100%",
          padding: "8px 12px",
          fontSize: "14px",
          border: "1px solid #444",
          borderRadius: "4px",
          backgroundColor: "#1e1e1e",
          color: "#fff",
        }}
      />
    </div>
  );
}
