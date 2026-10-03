/** Reattach retained views after their transport recovers, without replacing local drafts. */
export interface ConnectionRecovery {
  subscribe(restore: () => Promise<void>): () => void;
}

let recovery: ConnectionRecovery = { subscribe: () => () => {} };
export const useConnectionRecovery = (source: ConnectionRecovery) => { recovery = source; };
export const onConnectionRestored = (restore: () => Promise<void>) => recovery.subscribe(restore);
