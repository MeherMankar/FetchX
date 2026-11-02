interface EmptyStateProps {
  title: string;
  subtitle?: string;
}

export function EmptyState({ title, subtitle }: EmptyStateProps) {
  return (
    <div className="empty-state" role="status">
      <img src="/fetchx-logo.png" className="empty-logo" alt="" aria-hidden="true" width={64} height={64} />
      <strong>{title}</strong>
      {subtitle && <span>{subtitle}</span>}
    </div>
  );
}
