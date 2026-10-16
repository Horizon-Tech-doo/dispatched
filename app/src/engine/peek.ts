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
  /** The first page is on its way. */
  loading: boolean;
  /** Further pages are on their way. */
  streaming: boolean;
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
  /** A continuation exists (never from the end). */
  canPeekMore: boolean;
  /** Read another Top N, with a fresh scan budget. */
  peekMore(): void;
  /** Ask for no further pages; the rows already loaded stay. */
  stop(): void;
}

interface Param { from: PeekFrom }
interface Round { key: string; rows: number; scanned: number; stopped: boolean }

const loadedIn = (data: InfiniteData<PeekPageView, Param> | undefined) => (data?.pages ?? []).reduce((n, p) => n + p.messages.length, 0);

/** Peek a view in pages of summaries, one page at a time, continuing until Top N, the scan budget, the end, or
 *  Stop. A page that arrives for an older key (another entity, view, filter or start) never reaches this one. */
export function usePeekStream(o: PeekStreamOptions): PeekStream {
  const tail = o.from.kind === "end";
  const budget = o.scanBudget ?? SCAN_BUDGET;
  const tailN = tail ? Math.max(1, Math.min(o.topN, TAIL_MAX)) : null;
  const key = keys.peek(o.profileId, o.entity, o.view, o.from, o.filter, tailN);
  const keyText = hashKey(key);
  // A round is one Top N: it starts with the key and again with each Peek more.
  const [round, setRound] = useState<Round>({ key: keyText, rows: 0, scanned: 0, stopped: false });
  const current: Round = round.key === keyText ? round : { key: keyText, rows: 0, scanned: 0, stopped: false };
  if (current !== round) setRound(current);
  // Read by the query function, which asks only for what this round still needs.
  const target = useRef(0);
  target.current = current.rows + o.topN;

  const q = useInfiniteQuery<PeekPageView, EngineError, InfiniteData<PeekPageView, Param>, typeof key, Param>({
    queryKey: key,
    initialPageParam: { from: o.from },
    queryFn: ({ pageParam, client, queryKey }) => {
      const loaded = loadedIn(client.getQueryData<InfiniteData<PeekPageView, Param>>(queryKey));
      const max = tailN ?? Math.max(1, Math.min(PAGE_SIZE, target.current - loaded));
      return commands.peekPage(o.profileId!, { entity: o.entity, view: o.view, from: pageParam.from, max, filter: o.filter });
    },
    getNextPageParam: (last) => (tail || last.next_sequence == null ? undefined : { from: { kind: "sequence", sequence: last.next_sequence } }),
    enabled: (o.enabled ?? true) && o.profileId !== null,
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });

  const data = q.data;
  const messages = useMemo(() => data?.pages.flatMap((p) => p.messages) ?? [], [data]);
  const scanned = useMemo(() => data?.pages.reduce((n, p) => n + p.scanned, 0) ?? 0, [data]);
  const roundScanned = scanned - current.scanned;
  const below = messages.length < target.current;
  const more = !tail && q.hasNextPage;
  const wantMore = more && !current.stopped && !q.isFetching && !q.isError && below && roundScanned < budget;
  const { fetchNextPage } = q;
  useEffect(() => {
    // One page at a time: never restart a page that is on its way.
    if (wantMore) void fetchNextPage({ cancelRefetch: false });
  }, [wantMore, fetchNextPage, messages.length]);

  const last = data?.pages.at(-1);
  return {
    messages,
    scanned,
    loading: q.isPending && q.fetchStatus !== "idle",
    streaming: q.isFetchingNextPage,
    error: q.error ?? null,
    exhausted: !!last?.exhausted,
    olderNotExamined: tail && !!last && last.exhausted && last.messages.length < tailN! && last.scanned >= last.scan_limit,
    budgetHit: more && below && roundScanned >= budget,
    stopped: current.stopped,
    canPeekMore: more && !q.isFetching,
    peekMore: () => setRound({ key: keyText, rows: messages.length, scanned, stopped: false }),
    stop: () => setRound({ ...current, stopped: true }),
  };
}
