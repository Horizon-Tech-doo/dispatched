import { call } from "./ipc";
import type {
  AccountView, AppInfo, ConnectionReport, ConnectionView, DiscoveryView, EntityListing, Profile, ProfileDraft, SignInView, SubscriptionInfo, TenantView,
} from "./types";

export const appInfo = () => call<AppInfo>("app_info");
export const settingsGet = (key: string) => call<unknown>("settings_get", { key });
export const settingsSet = (key: string, value: unknown) => call<void>("settings_set", { key, value });

export const profilesList = () => call<Profile[]>("profiles_list");
export const profileSave = (draft: ProfileDraft) => call<Profile>("profile_save", { draft });
export const profileDelete = (id: string) => call<void>("profile_delete", { id });
export const profileTest = (id: string) => call<ConnectionReport>("profile_test", { id });

export const accountCurrent = () => call<AccountView | null>("account_current");
/** Signs in. With a tenant other than the current account's home, the sign-in is that tenant's own and the account stays. */
export const signIn = (tenant: string | null, deviceCode: boolean) => call<SignInView>("sign_in", { tenant, deviceCode });
export const signInCancel = () => call<void>("sign_in_cancel");

/** The signed-in account's tenants, home first. */
export const listTenants = () => call<TenantView[]>("list_tenants");
/** Service Bus namespaces in one tenant (`null` is the home tenant). A guest tenant that needs multi-factor sign-in fails with kind `mfa_required`. */
export const discoverNamespaces = (tenant: string | null) => call<DiscoveryView>("discover_namespaces", { tenant });

export const connect = (profileId: string) => call<ConnectionView>("connect", { profileId });
export const disconnect = () => call<void>("disconnect");

/** Listings name the profile they are for; the engine refuses one that is no longer connected. */
export const listEntities = (profileId: string) => call<EntityListing>("list_entities", { profileId });
export const listSubscriptions = (profileId: string, topic: string) => call<SubscriptionInfo[]>("list_subscriptions", { profileId, topic });
