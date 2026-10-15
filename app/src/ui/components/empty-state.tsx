import { CircleAlert } from "lucide-react";
import type { ReactNode } from "react";

export function EmptyState({
  title,
  body,
  action,
  tone = "neutral",
}: {
  title: string;
  body?: ReactNode;
  action?: ReactNode;
  tone?: "neutral" | "error";
}) {
  return (
    <div className="flex max-w-[440px] flex-col gap-1.5 px-4 py-5">
      <h3 className="m-0 flex items-center gap-1.5 text-13 font-semibold text-fg">
        {tone === "error" && <CircleAlert aria-hidden="true" className="size-3.5 text-accent" />}
        {title}
      </h3>
      {body && <div className="text-12 text-muted">{body}</div>}
      {action && <div className="pt-1.5">{action}</div>}
    </div>
  );
}
