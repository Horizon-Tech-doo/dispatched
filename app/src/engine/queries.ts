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

/** Queues and topics of the connected profile. `refetchMs` 0 disables auto-refresh. */
export function useEntities(profileId: string | null, refetchMs = 0) {
  return useQuery({
    queryKey: keys.entities(profileId),
    queryFn: commands.listEntities,
    enabled: profileId !== null,
    refetchInterval: refetchMs > 0 ? refetchMs : false,
    placeholderData: (prev) => prev,
  });
}

/** Subscriptions for each expanded topic, loaded lazily and independently. */
export function useSubscriptionsFor(profileId: string | null, topics: string[], refetchMs = 0) {
  return useQueries({
    queries: topics.map((topic) => ({
      queryKey: keys.subscriptions(profileId, topic),
      queryFn: () => commands.listSubscriptions(topic),
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
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.profiles }),
  });
}

export function useDeleteProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.profileDelete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.profiles }),
  });
}

export const useTestProfile = () => useMutation({ mutationFn: (id: string) => commands.profileTest(id) });

export function useSignIn() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ tenant, deviceCode }: { tenant: string | null; deviceCode: boolean }) => commands.signIn(tenant, deviceCode),
    onSuccess: (account) => qc.setQueryData(keys.account, account),
  });
}
