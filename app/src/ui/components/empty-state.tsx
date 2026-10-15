import { CircleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

interface MessageProps {
  title: string;
  /** What happened and what to do, in plain words. */
  body?: ReactNode;
  /** The raw machine detail (an engine message), on its own line in mono. */
  detail?: string;
  action?: ReactNode;
  className?: string;
  /** `alert` for a problem that appears while the user is looking (an update that failed). */
  role?: "alert";
}

function Message({ title, body, detail, action, className, role, error }: MessageProps & { error: boolean }) {
  return (
    <div role={role} className={cn("flex flex-col gap-1.5", className)}>
      <h3 className="m-0 flex items-start gap-1.5 text-13 font-semibold text-fg">
        {error && <CircleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-accent" />}
        <span className="min-w-0">{title}</span>
      </h3>
      {body && <div className="text-12 text-muted">{body}</div>}
      {detail && (
        <p className="m-0 break-words font-mono text-11 text-muted" data-selectable>
          {detail}
        </p>
      )}
      {action && <div className="flex flex-wrap gap-2 pt-1.5">{action}</div>}
    </div>
  );
}

/** The one error form: amber icon and title, a plain sentence, the raw detail beneath, then the way out.
 *  Used on its own in banners and sheets, and inside `EmptyState` for whole-pane errors. */
export function Problem(props: MessageProps) {
  return <Message {...props} error />;
}

export function EmptyState({
  tone = "neutral",
  ...props
}: MessageProps & {
  tone?: "neutral" | "error";
}) {
  return <Message {...props} error={tone === "error"} className={cn("max-w-[440px] px-4 py-5", props.className)} />;
}
