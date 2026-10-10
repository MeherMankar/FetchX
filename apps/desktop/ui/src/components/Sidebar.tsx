import type { NavSection } from "../App";
import type { StatusCounts } from "../lib/status";

interface SidebarProps {
  active: NavSection;
  counts: StatusCounts;
  onSelect: (section: NavSection) => void;
}

interface NavItem {
  section: NavSection;
  label: string;
  count?: number;
}

export function Sidebar({ active, counts, onSelect }: SidebarProps) {
  const items: NavItem[] = [
    { section: "all", label: "All Downloads", count: counts.total },
    { section: "active", label: "Active", count: counts.active },
    { section: "queued", label: "Queued", count: counts.queued },
    { section: "paused", label: "Paused", count: counts.paused },
    { section: "scheduled", label: "Scheduled", count: counts.scheduled },
    { section: "completed", label: "Completed", count: counts.completed },
    { section: "failed", label: "Failed", count: counts.failed },
    { section: "cancelled", label: "Cancelled", count: counts.cancelled },
  ];

  return (
    <nav className="sidebar" aria-label="Download categories">
      <div className="sidebar-brand" aria-hidden="true">
        <span className="brand-mark">↓</span>
        <span className="brand-name">MyDownloader</span>
      </div>

      <ul className="nav-list" role="list">
        {items.map((item) => (
          <li key={item.section}>
            <button
              className={`nav-item${active === item.section ? " nav-item--active" : ""}`}
              onClick={() => onSelect(item.section)}
              aria-current={active === item.section ? "page" : undefined}
            >
              <span className="nav-label">{item.label}</span>
              {item.count !== undefined && item.count > 0 && (
                <span className="nav-count" aria-label={`${item.count} items`}>
                  {item.count}
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
