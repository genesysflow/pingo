import type { FunctionReference, Id } from '@rxferrix/client';
import { api as generated } from '../rxferrix/api';
import { api as mock } from './mock-api';

/** Routes and mock fixtures carry plain IDs; all fields still come from codegen. */
export type Plain<T> = T extends Id<string> ? string
  : T extends FunctionReference<infer K, infer A, infer O> ? FunctionReference<K, Plain<A>, Plain<O>>
  : T extends readonly (infer V)[] ? Plain<V>[]
  : T extends object ? { [K in keyof T]: Plain<T[K]> } : T;
export const api = (import.meta.env.VITE_PINGO_MOCK === '1' ? mock : generated) as Plain<typeof generated>;
