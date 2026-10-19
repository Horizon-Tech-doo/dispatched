import { subscribe } from "./ipc";
import type { ActivityEntry, ActivityRecord, ConfirmRequest, JobEvent, SignInPrompt } from "./types";

export const onActivity = (cb: (e: ActivityEntry) => void) => subscribe<ActivityEntry>("activity", cb);
export const onJob = (cb: (e: JobEvent) => void) => subscribe<JobEvent>("job", cb);
export const onSignInPrompt = (cb: (p: SignInPrompt) => void) => subscribe<SignInPrompt>("sign-in-prompt", cb);
/** A job asks to be confirmed. */
export const onConfirm = (cb: (c: ConfirmRequest) => void) => subscribe<ConfirmRequest>("confirm", cb);
/** An entry written to the shared activity log by this app. */
export const onActivityRecord = (cb: (r: ActivityRecord) => void) => subscribe<ActivityRecord>("activity-record", cb);
