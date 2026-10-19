import { hashKey, useInfiniteQuery, type InfiniteData } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import * as commands from "./commands";
import type { EngineError } from "./ipc";
import { keys } from "./queries";
import type { FilterSpec, MessageSummary, PeekFrom, PeekPageView, PeekView } from "./types";

/** Summaries per page (the engine's cap). */
export const PAGE_SIZE = 500;
/** Most messages "from the end" returns. */
export const TAIL_MAX = 5_000;
/** Most messages one load examines before asking the user to continue. */
export const SCAN_BUDGET = 100_000;
/** Most messages one page call examines (the engine's cap). */
const SCAN_PER_PAGE = 5_000;

export interface PeekStreamOptions {
  profileId: string | null;
  entity: string;
  view: PeekView;
  from: PeekFrom;
  /** Matches to load per round ("Top N"). */
  topN: number;
  filter: FilterSpec;
  enabled?: boolean;
  /** Tests lower it; the app uses SCAN_BUDGET. */
  scanBudget?: number;
}

export interface PeekStream {
  messages: MessageSummary[];
  /** Messages examined by every page so far. */
  scanned: number;
  /** The rows this round reads up to: the rows before it plus Top N (from the end: Top N). */
  target: number;
  /** The first page is on its way. */
  loading: boolean;
  /** Further pages are on their way. */
  streaming: boolean;
  /** The last page call failed; the rows already loaded stay. */
  error: EngineError | null;
  /** Forward: the end of the view was reached. From the end: the scan covered the newest messages (false means
   *  the budget ran out first, so the rows are not the newest). */
  exhausted: boolean;
  /** From the end only: fewer matches than asked and the scan stopped at its limit, so older messages may not
   *  have been examined. */
  olderNotExamined: boolean;
  /** This round examined the scan budget without reaching Top N. */
  budgetHit: boolean;
  stopped: boolean;
  /** A continuation exists (never from the end), also after a failed page. */
  canPeekMore: boolean;
  /** The last page failed and can be asked for again. */
  canRetry: boolean;
  /** Read another Top N, with a fresh scan budget (after a failed page too). */
  peekMore(): void;
  /** Ask again for the page that failed and continue the round (the first page: load it again). */
  retry(): void;
  /** Ask for no further pages; the rows already loaded stay. */
  stop(): void;
}

/** What one page call asks for. Kept with each page, so a refetch of the whole stream asks for the same sizes. */
interface Param { from: PeekFrom; max: number; scanLimit: number | null }
/** One round is one Top N. `first` is the first page when the round was set by Peek more or Stop: when the query
 *  no longer holds that page (a reset, new data), the next round starts fresh. */
interface Round { key: string; rows: number; scanned: number; stopped: boolean; first?: PeekPageView }
/** What the next page may ask for: this round's target rows, and the scan budget from `baseline` on. */
interface Plan { target: number; baseline: number; budget: number }

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
const sum = (pages: PeekPageView[], f: (p: PeekPageView) => number) => pages.reduce((n, p) => n + f(p), 0);

/** Peek a view in pages of summaries, one page at a time, continuing until Top N, the scan budget, the end, or
 *  Stop. A page that arrives for an older key (another entity, view, filter or start) never reaches this one. */
export function usePeekStream(o: PeekStreamOptions): PeekStream {
  const tail = o.from.kind === "end";
  const budget = Math.max(1, o.scanBudget ?? SCAN_BUDGET);
  const tailN = tail ? clamp(o.topN, 1, TAIL_MAX) : null;
  const key = keys.peek(o.profileId, o.entity, o.view, o.from, o.filter, tailN);
  const keyText = hashKey(key);
  const fresh: Round = { key: keyText, rows: 0, scanned: 0, stopped: false };
  const [round, setRound] = useState<Round>(fresh);
  // Read when a page is about to be asked for (`getNextPageParam` runs at fetch time).
  const plan = useRef<Plan>({ target: o.topN, baseline: 0, budget });

  const q = useInfiniteQuery<PeekPageView, EngineError, InfiniteData<PeekPageView, Param>, typeof key, Param>({
    queryKey: key,
    initialPageParam: { from: o.from, max: tailN ?? clamp(o.topN, 1, PAGE_SIZE), scanLimit: tail ? null : Math.min(SCAN_PER_PAGE, budget) },
    queryFn: ({ pageParam }) =>
      commands.peekPage(o.profileId!, { entity: o.entity, view: o.view, from: pageParam.from, max: pageParam.max, scan_limit: pageParam.scanLimit, filter: o.filter }),
    getNextPageParam: (last, pages) => {
      if (tail || last.next_sequence == null) return undefined;
      const p = plan.current;
      return {
        from: { kind: "sequence", sequence: last.next_sequence },
        max: clamp(p.target - sum(pages, (x) => x.messages.length), 1, PAGE_SIZE),
        scanLimit: clamp(p.budget - (sum(pages, (x) => x.scanned) - p.baseline), 1, SCAN_PER_PAGE),
      };
    },
    enabled: (o.enabled ?? true) && o.profileId !== null,
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });

  const data = q.data;
  const messages = useMemo(() => data?.pages.flatMap((p) => p.messages) ?? [], [data]);
  const scanned = useMemo(() => (data ? sum(data.pages, (p) => p.scanned) : 0), [data]);
  // Another key, or the pages this round was counted from are gone (a reset query): a fresh round at Top N.
  const gone = round.first !== undefined && data?.pages[0] !== round.first;
  const current = round.key !== keyText || gone || scanned < round.scanned ? fresh : round;
  if (current !== round) setRound(current);
  const target = current.rows + o.topN;
  plan.current = { target, baseline: current.scanned, budget };

  const roundScanned = scanned - current.scanned;
  const below = messages.length < target;
  const more = !tail && q.hasNextPage;
  const wantMore = more && !current.stopped && !q.isFetching && !q.isError && below && roundScanned < budget;
  const { fetchNextPage, refetch } = q;
  useEffect(() => {
    // One page at a time: never restart a page that is on its way. Runs again after every page (`data`), also
    // one with no matches.
    if (wantMore) void fetchNextPage({ cancelRefetch: false });
  }, [wantMore, fetchNextPage, data]);

  /** Ask for the next page now (the effect waits while the query is in error). */
  const resume = (next: Round) => {
    plan.current = { target: next.rows + o.topN, baseline: next.scanned, budget };
    if (q.isError && !q.isFetching) void fetchNextPage({ cancelRefetch: false });
  };

  const last = data?.pages.at(-1);
  return {
    messages,
    scanned,
    target: tail ? tailN! : target,
    loading: q.isPending && q.fetchStatus !== "idle",
    streaming: q.isFetchingNextPage,
    error: q.error ?? null,
    exhausted: !!last?.exhausted,
    olderNotExamined: tail && !!last && last.exhausted && last.messages.length < tailN! && last.scanned >= last.scan_limit,
    budgetHit: more && below && roundScanned >= budget,
    stopped: current.stopped,
    canPeekMore: more && !q.isFetching,
    canRetry: q.isError && !q.isFetching,
    peekMore: () => {
      const next: Round = { key: keyText, rows: messages.length, scanned, stopped: false, first: data?.pages[0] };
      setRound(next);
      if (more) resume(next);
    },
    retry: () => {
      if (!q.isError || q.isFetching) return;
      if (more) {
        const next: Round = { ...current, stopped: false, first: data?.pages[0] };
        setRound(next);
        resume(next);
      } else void refetch();
    },
    stop: () => setRound({ ...current, stopped: true, first: data?.pages[0] }),
  };
}
