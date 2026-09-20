// Shared by IPC and the store without a circular import.
let workspace: string | null = null;
export const currentWorkspace = () => workspace;
export const setCurrentWorkspace = (next: string | null) => { workspace = next; };
