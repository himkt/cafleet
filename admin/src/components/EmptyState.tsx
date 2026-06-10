import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  children?: ReactNode;
}

export default function EmptyState({
  icon: Icon,
  title,
  children,
}: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
      <Icon size={24} className="text-text-faint" aria-hidden="true" />
      <p className="text-sm text-text-muted">{title}</p>
      {children}
    </div>
  );
}
