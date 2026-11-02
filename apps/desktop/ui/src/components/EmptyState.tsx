interface EmptyStateProps {
  title: string;
  subtitle?: string;
}

export function EmptyState({ title, subtitle }: EmptyStateProps) {
  return (
    <div className="empty-state" role="status">
      <div className="empty-logo-wrap">
        <img src="/fetchx-logo.png" className="empty-logo" alt="" aria-hidden="true" />
      </div>
      <strong>{title}</strong>
      {subtitle && <span>{subtitle}</span>}
    </div>
  );
}
