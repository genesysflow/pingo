// Production switch point: replace these exports with generated api + @rxferrix/svelte.
export { api } from './api';
export { useQuery, useMutation, usePaginatedQuery, useAuth, useCurrentUser, uploadFile } from './hooks.svelte';
export type * from './types';

export { insertAtTop, getAllPages } from '@rxferrix/client';
