import { Fragment, type ReactNode } from "react";

/** `text` with every occurrence of `term` (a host name) kept on one line, so a sentence never breaks inside it. */
export function keepTogether(text: string, term: string): ReactNode {
  if (!term || !text.includes(term)) return text;
  const parts = text.split(term);
  return parts.map((part, i) => (
    <Fragment key={i}>
      {part}
      {i < parts.length - 1 && <span className="whitespace-nowrap">{term}</span>}
    </Fragment>
  ));
}
