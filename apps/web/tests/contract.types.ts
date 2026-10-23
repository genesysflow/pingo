// Compiled by svelte-check: the mock hook boundary must accept the real library's signatures.
import type * as Real from '@rxferrix/svelte';
import {
  useQuery,
  useMutation,
  usePaginatedQuery,
  useAuth,
  useCurrentUser,
  uploadFile,
} from '../src/lib/data';
export const query: typeof Real.useQuery = useQuery;
export const mutation: typeof Real.useMutation = useMutation;
export const paginated: typeof Real.usePaginatedQuery = usePaginatedQuery;
export const auth: typeof Real.useAuth = useAuth;
export const currentUser: typeof Real.useCurrentUser = useCurrentUser;
export const upload: typeof Real.uploadFile = uploadFile;
