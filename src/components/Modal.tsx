import type { ReactNode } from "react";
import { Icon } from "./Icons";

export function Modal({
  title,
  subtitle,
  onClose,
  children,
  footer,
  width = "max-w-2xl",
}: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={onClose}
      />
      <div
        className={`panel relative z-10 flex max-h-[88vh] w-full ${width} flex-col overflow-hidden rounded-2xl shadow-2xl`}
      >
        <div className="flex items-start justify-between gap-4 border-b border-line px-5 py-4">
          <div>
            <h2 className="text-[15px] font-semibold text-ink">{title}</h2>
            {subtitle && (
              <p className="mt-0.5 text-xs text-ink-muted">{subtitle}</p>
            )}
          </div>
          <button className="btn btn-ghost !p-1.5" onClick={onClose} aria-label="Close">
            <Icon name="x" />
          </button>
        </div>
        <div className="scroll flex-1 overflow-y-auto px-5 py-4">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-line px-5 py-3">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}
