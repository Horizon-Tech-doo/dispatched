import { subscribe } from "./ipc";
import type { ActivityEntry, JobEvent, SignInPrompt } from "./types";

export const onActivity = (cb: (e: ActivityEntry) => void) => subscribe<ActivityEntry>("activity", cb);
export const onJob = (cb: (e: JobEvent) => void) => subscribe<JobEvent>("job", cb);
export const onSignInPrompt = (cb: (p: SignInPrompt) => void) => subscribe<SignInPrompt>("sign-in-prompt", cb);
