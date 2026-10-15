import { QueryClient, useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import * as commands from "./commands";
import type { EngineError } from "./ipc";
import type { ProfileDraft } from "./types";

export const keys = {
  appInfo: ["app-info"] as const,
  profiles: ["profiles"] as const,
  account: ["account"] as const,
  entities: (profileId: string | null) => ["entities", profileId] as const,
  subscriptions: (profileId: string | null, topic: string) => ["subscriptions", profileId, topic] as const,
  tenants: (accountId: string | null) => ["tenants", accountId] as const,
  discovery: (accountId: string | null, tenantId: string | null) => ["discovery", accountId, tenantId] as const,
  setting: (key: string) => ["setting", key] as const,
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
    },
  });
}
