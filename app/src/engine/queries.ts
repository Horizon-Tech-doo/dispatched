import { QueryClient, useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import * as commands from "./commands";
import type { EngineError } from "./ipc";
import type { EntityKindName, ExportRequest, FilterSpec, PeekFrom, PeekView, ProfileDraft } from "./types";

export const keys = {
  appInfo: ["app-info"] as const,
  profiles: ["profiles"] as const,
  account: ["account"] as const,
  entities: (profileId: string | null) => ["entities", profileId] as const,
  subscriptions: (profileId: string | null, topic: string) => ["subscriptions", profileId, topic] as const,
  tenants: (accountId: string | null) => ["tenants", accountId] as const,
  discovery: (accountId: string | null, tenantId: string | null) => ["discovery", accountId, tenantId] as const,
  setting: (key: string) => ["setting", key] as const,
  /** A peek stream; `tailN` is the "from the end" count (null for forward peeks, whose Top N is not part of the key). */
  peek: (profileId: string | null, entity: string, view: PeekView, from: PeekFrom, filter: FilterSpec, tailN: number | null) =>
    ["peek", profileId, entity, view, from, filter, tailN] as const,
  message: (profileId: string | null, entity: string, view: PeekView, sequence: number | null) => ["message", profileId, entity, view, sequence] as const,
  details: (profileId: string | null, kind: EntityKindName, entity: string) => ["entity-details", profileId, kind, entity] as const,
  rules: (profileId: string | null, entity: string) => ["rules", profileId, entity] as const,
  sessions: (profileId: string | null, entity: string) => ["sessions", profileId, entity] as const,
  dashboard: (profileId: string | null) => ["dashboard", profileId] as const,
  filterCheck: (filter: FilterSpec) => ["filter-check", filter] as const,
  signIns: ["sign-ins"] as const,
};

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        staleTime: 30_000,
        retry: (count, e) => count < 1 && (e as EngineError).kind === "network",
      },
    },
  });
}

export const useAppInfo = () => useQuery({ queryKey: keys.appInfo, queryFn: commands.appInfo, staleTime: Infinity });
export const useProfiles = () => useQuery({ queryKey: keys.profiles, queryFn: commands.profilesList });
export const useAccount = () => useQuery({ queryKey: keys.account, queryFn: commands.accountCurrent });

/** A listing that needs the account: when it says the sign-in is gone, the account is read again, so a
 *  sheet that showed a picker shows the sign-in panel instead. */
function needsAccount<T>(qc: QueryClient, run: () => Promise<T>): () => Promise<T> {
  return async () => {
    try {
      return await run();
    } catch (e) {
      if ((e as EngineError).kind === "not_signed_in") void qc.invalidateQueries({ queryKey: keys.account });
      throw e;
    }
  };
}

/** The tenants of the signed-in account; another account refetches. */
export function useTenants(enabled = true) {
  const qc = useQueryClient();
  const accountId = useAccount().data?.id ?? null;
  return useQuery({
    queryKey: keys.tenants(accountId),
    queryFn: needsAccount(qc, commands.listTenants),
    enabled: enabled && accountId !== null,
  });
}

/** Service Bus namespaces in a tenant (`null` is the home tenant) of the signed-in account. */
export function useDiscoveredNamespaces(tenantId: string | null, enabled = true) {
  const qc = useQueryClient();
  const accountId = useAccount().data?.id ?? null;
  return useQuery({
    queryKey: keys.discovery(accountId, tenantId),
    queryFn: needsAccount(qc, () => commands.discoverNamespaces(tenantId)),
    enabled: enabled && accountId !== null,
  });
}

/** Queues and topics of the connected profile. `refetchMs` 0 disables auto-refresh. */
export function useEntities(profileId: string | null, refetchMs = 0) {
  return useQuery({
    queryKey: keys.entities(profileId),
    queryFn: () => commands.listEntities(profileId!),
    enabled: profileId !== null,
    refetchInterval: refetchMs > 0 ? refetchMs : false,
    // Keep showing the last listing while the same profile reloads, never another profile's.
    placeholderData: (prev, prevQuery) => (prevQuery?.queryKey[1] === profileId ? prev : undefined),
  });
}

/** Subscriptions for each expanded topic, loaded lazily and independently. */
export function useSubscriptionsFor(profileId: string | null, topics: string[], refetchMs = 0) {
  return useQueries({
    queries: topics.map((topic) => ({
      queryKey: keys.subscriptions(profileId, topic),
      queryFn: () => commands.listSubscriptions(profileId!, topic),
      enabled: profileId !== null,
      refetchInterval: refetchMs > 0 ? refetchMs : (false as const),
    })),
  });
}

export function useSetting(key: string) {
  return useQuery({ queryKey: keys.setting(key), queryFn: () => commands.settingsGet(key), staleTime: Infinity });
}

export function useSaveProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (draft: ProfileDraft) => commands.profileSave(draft),
    // The draft can carry a connection string: do not keep it in the mutation cache after the call.
    gcTime: 0,
    // Pinned entities can change what the tree lists.
    onSuccess: () => Promise.all([qc.invalidateQueries({ queryKey: keys.profiles }), qc.invalidateQueries({ queryKey: ["entities"] })]),
  });
}

export function useDeleteProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.profileDelete(id),
    onSuccess: (_, id) => {
      qc.removeQueries({ queryKey: keys.entities(id) });
      qc.removeQueries({ queryKey: ["subscriptions", id] });
      return qc.invalidateQueries({ queryKey: keys.profiles });
    },
  });
}

export const useTestProfile = () => useMutation({ mutationFn: (id: string) => commands.profileTest(id) });

export function useSignIn() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ tenant, deviceCode }: { tenant: string | null; deviceCode: boolean }) => commands.signIn(tenant, deviceCode),
    onSuccess: (r) => {
      qc.setQueryData(keys.account, r.account);
      // A new sign-in changes which tenants have their own sign-in and whether profiles' sign-ins work.
      void qc.invalidateQueries({ queryKey: ["tenants"] });
      void qc.invalidateQueries({ queryKey: keys.profiles });
      void qc.invalidateQueries({ queryKey: keys.signIns });
    },
  });
}

/** One message in full, by sequence number; `null` data when it is no longer in the view. */
export function useMessageDetail(profileId: string | null, entity: string, view: PeekView, sequence: number | null) {
  return useQuery({
    queryKey: keys.message(profileId, entity, view, sequence),
    queryFn: () => commands.messageDetail(profileId!, entity, view, sequence!),
    enabled: profileId !== null && sequence !== null,
    staleTime: Infinity,
    gcTime: 60_000,
  });
}

/** An entity's details. `refetchMs` 0 disables polling (Overview polls every 10 s; the engine logs only failures). */
export function useEntityDetails(profileId: string | null, kind: EntityKindName, entity: string, refetchMs = 0) {
  return useQuery({
    queryKey: keys.details(profileId, kind, entity),
    queryFn: () => commands.entityDetails(profileId!, kind, entity),
    enabled: profileId !== null,
    refetchInterval: refetchMs > 0 ? refetchMs : false,
    // Keep showing the last details only while the same entity reloads, never another entity's or profile's.
    placeholderData: (prev, prevQuery) => {
      const k = prevQuery?.queryKey;
      return k?.[1] === profileId && k?.[2] === kind && k?.[3] === entity ? prev : undefined;
    },
  });
}

export const useRules = (profileId: string | null, entity: string, enabled = true) =>
  useQuery({ queryKey: keys.rules(profileId, entity), queryFn: () => commands.listRules(profileId!, entity), enabled: enabled && profileId !== null });

export const useSessions = (profileId: string | null, entity: string, enabled = true) =>
  useQuery({ queryKey: keys.sessions(profileId, entity), queryFn: () => commands.listSessions(profileId!, entity), enabled: enabled && profileId !== null });

/** The Dashboard. `refetchMs` 0 disables auto-refresh. */
export const useDashboard = (profileId: string | null, refetchMs = 0) =>
  useQuery({
    queryKey: keys.dashboard(profileId),
    queryFn: () => commands.dashboard(profileId!),
    enabled: profileId !== null,
    refetchInterval: refetchMs > 0 ? refetchMs : false,
  });

const CHECKED: (keyof FilterSpec)[] = ["property", "body_regex", "body_json_path", "body_json_equals"];
/** No field that can have a problem is set (the engine treats a field blank after a trim as no condition). */
const nothingToCheck = (f: FilterSpec) => CHECKED.every((k) => !f[k]?.trim());

/** Problems with a filter, checked by the engine. A filter with nothing to check needs no call (`data` stays
 *  undefined: no issues). */
export const useFilterCheck = (filter: FilterSpec) =>
  useQuery({
    queryKey: keys.filterCheck(filter),
    queryFn: () => commands.checkFilter(filter),
    enabled: !nothingToCheck(filter),
    staleTime: Infinity,
    gcTime: 30_000,
  });

/** Starts an export with a pick from `commands.pickExportPath`; progress arrives on `onJob`. */
export const useStartExport = () =>
  useMutation({ mutationFn: ({ profileId, request }: { profileId: string; request: ExportRequest }) => commands.exportStart(profileId, request) });

export const useSignIns = () => useQuery({ queryKey: keys.signIns, queryFn: commands.signInsList });

/** Signs a stored sign-in out. The result's `closed_profile_id` names the connection the engine closed because
 *  it used that sign-in: `onClosed` lets the caller reset its connection state for it. */
export function useSignOut(onClosed?: (profileId: string) => void) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.signOut(id),
    // Hook-level, so it still runs when the component that started the sign-out has gone.
    // Signing out changes which profiles can connect (`bound_sign_in.live`), the tenants and the discovery too.
    onSuccess: (r) => {
      if (r.closed_profile_id !== null) onClosed?.(r.closed_profile_id);
      return Promise.all(
        [keys.signIns, keys.account, keys.profiles, ["tenants"], ["discovery"]].map((queryKey) => qc.invalidateQueries({ queryKey })),
      );
    },
  });
}

/** The draft can carry a connection string: never kept in the mutation cache. */
export const useTestDraft = () => useMutation({ mutationFn: (draft: ProfileDraft) => commands.profileTestDraft(draft), gcTime: 0 });
