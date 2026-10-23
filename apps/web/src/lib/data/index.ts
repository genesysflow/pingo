import * as live from '@rxferrix/svelte';
import * as mock from './hooks.svelte';
export { api } from './api';
export type * from './types';
export { insertAtTop, getAllPages } from '@rxferrix/client';
export { errorMessage } from './errors';

/** Explicit opt-in keeps design previews independent of a server. */
export const isDemo = import.meta.env.VITE_PINGO_MOCK === '1';
export const useQuery: typeof live.useQuery = isDemo ? mock.useQuery : live.useQuery;
export const useMutation: typeof live.useMutation = isDemo ? mock.useMutation : live.useMutation;
export const usePaginatedQuery: typeof live.usePaginatedQuery = isDemo ? mock.usePaginatedQuery : live.usePaginatedQuery;
export const useAuth: typeof live.useAuth = isDemo ? mock.useAuth : live.useAuth;
export const useCurrentUser: typeof live.useCurrentUser = isDemo ? mock.useCurrentUser : live.useCurrentUser;
export const uploadFile: typeof live.uploadFile = isDemo ? mock.uploadFile : live.uploadFile;
