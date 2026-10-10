interface EmptyStateProps {
  icon?: string;
  title: string;
  subtitle?: string;
}

export function EmptyState({ icon = "↓", title, subtitle }: EmptyStateProps) {
  return (
    <div className="empty-state" role="status">
      <span className="empty-icon" aria-hidden="true">{icon}</span>
      <strong>{title}</strong>
      {subtitle && <span>{subtitle}</span>}
    </div>
  );
}
