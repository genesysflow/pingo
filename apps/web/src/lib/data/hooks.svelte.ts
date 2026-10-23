import { createSubscriber } from 'svelte/reactivity';
import { untrack } from 'svelte';
import type {
  ArgsOf,
  FunctionReference,
  OutputOf,
  PaginationOpts,
  PaginationResult,
  RxFerrixClient,
  OptionalArgs,
  OptimisticUpdate,
  Id,
  User,
  RxFerrixAuth,
  PaginatedQueryReference,
  PaginatedQueryArgs,
  PaginatedQueryItem,
  UploadFileOptions,
} from '@rxferrix/client';
import type {
  UseQueryResult,
  UseQueryOptions,
  Mutation,
  UsePaginatedQueryResult,
  UsePaginatedQueryOptions,
  UseAuthResult,
} from '@rxferrix/svelte';
import { MockBackend } from './backend';
import type { MessageView } from './types';
export const backend = new MockBackend();
const observe = createSubscriber((update) => backend.subscribe(update));
export function useQuery<Q extends FunctionReference<'query'>>(
  ref: Q,
  args: () => ArgsOf<Q> | 'skip' = () => ({}) as ArgsOf<Q>,
  _options: UseQueryOptions = {},
): UseQueryResult<OutputOf<Q>> {
  function read() {
    observe();
    const a = args();
    if (a === 'skip') return { data: undefined, error: undefined };
    try {
      return { data: backend.query(ref._path, a) as OutputOf<Q>, error: undefined };
    } catch (e) {
      return { data: undefined, error: e as UseQueryResult<OutputOf<Q>>['error'] };
    }
  }
  return {
    get data() {
      return read().data;
    },
    get error() {
      return read().error;
    },
    get isLoading() {
      return false;
    },
  };
}
export function useMutation<M extends FunctionReference<'mutation'>>(
  ref: M,
  _options: { client?: RxFerrixClient } = {},
): Mutation<M> {
  // Writes are synchronous in memory, so the optimistic result is visible before the promise resolves.
  const mutate = ((...args: OptionalArgs<M>) => {
    try {
      return Promise.resolve(backend.mutate(ref._path, args[0]) as OutputOf<M>);
    } catch (e) {
      return Promise.reject(e);
    }
  }) as Mutation<M>;
  mutate.withOptimisticUpdate = (_update: OptimisticUpdate<ArgsOf<M>>) => mutate;
  return mutate;
}
export function usePaginatedQuery<Q extends PaginatedQueryReference>(
  ref: Q,
  args: () => PaginatedQueryArgs<Q> | 'skip',
  options: UsePaginatedQueryOptions,
): UsePaginatedQueryResult<PaginatedQueryItem<Q>> {
  type Item = PaginatedQueryItem<Q>;
  let pages = $state<{ key: string; opts: PaginationOpts[] }>({ key: '', opts: [] });
  const key = $derived(JSON.stringify(args()));
  const initial = (): PaginationOpts[] => [{ cursor: null, numItems: options.initialNumItems }];
  $effect(() => {
    const next = key;
    untrack(() => {
      if (pages.key !== next) pages = { key: next, opts: initial() };
    });
  });
  function read() {
    observe();
    const a = args();
    if (a === 'skip') return { results: [] as Item[], tail: undefined, error: undefined };
    try {
      const opts = pages.key === key ? pages.opts : initial();
      const loaded = opts.map(
        (paginationOpts) => backend.query(ref._path, { ...a, paginationOpts }) as PaginationResult<Item>,
      );
      return { results: loaded.flatMap((page) => page.page), tail: loaded.at(-1), error: undefined };
    } catch (error) {
      return {
        results: [] as Item[],
        tail: undefined,
        error: error as UsePaginatedQueryResult<Item>['error'],
      };
    }
  }
  return {
    get results() {
      return read().results;
    },
    get status() {
      return args() === 'skip' ? 'LoadingFirstPage' : read().tail?.isDone ? 'Exhausted' : 'CanLoadMore';
    },
    get isLoading() {
      return args() === 'skip';
    },
    get error() {
      return read().error;
    },
    loadMore(n: number) {
      if (!Number.isInteger(n) || n < 1) throw new Error('Page size must be a positive integer.');
      const current = read();
      if (args() === 'skip' || !current.tail || current.tail.isDone) return;
      const opts = (pages.key === key ? pages.opts : initial()).map((page) => ({ ...page }));
      // Pin the previous page so live inserts grow it rather than push an older item off the end.
      opts[opts.length - 1].endCursor = current.tail.continueCursor;
      opts.push({ cursor: current.tail.continueCursor, numItems: n });
      pages = { key, opts };
    },
  };
}

export function useAuth(_options: { client?: RxFerrixClient } = {}): UseAuthResult {
  const signIn = async (_args: { email: string; password: string }) => {
    backend.authenticated = true;
    localStorage.setItem('pingo-session', 'demo');
    backend.notify();
    return { mfaRequired: false as const };
  };
  const unsupported = async () => {
    throw new Error('This account action is available with the live authentication service.');
  };
  const result = {
    get isAuthenticated() {
      observe();
      return backend.authenticated;
    },
    get isLoading() {
      return false;
    },
    get userId() {
      observe();
      return backend.authenticated ? ('you' as Id<'_users'>) : null;
    },
    get isAdmin() {
      return false;
    },
    signIn,
    async signUp(args: { email: string; password: string; name?: string }) {
      if (args.name) backend.state.people[0].name = args.name;
      await signIn(args);
      return { verificationRequired: false };
    },
    async signOut() {
      backend.authenticated = false;
      localStorage.setItem('pingo-session', 'signed-out');
      backend.notify();
    },
    async requestMagicLink(_args: { email: string; redirectTo?: string }) {
      /* The demo presents a local verification link. */
    },
    async verifyMagicLink(_token: string) {
      return signIn({ email: '', password: '' });
    },
    signInWithOAuth(_provider: string, _options?: { redirectTo?: string }) {
      void signIn({ email: '', password: '' });
    },
    async providers() {
      return { password: true, magicLink: true, signups: true, oauth: ['github', 'google', 'discord'] };
    },
    async handleRedirectCallback() {
      return null;
    },
    async getAccessToken() {
      return backend.authenticated ? 'demo-session' : null;
    },
    signInAsAdmin: unsupported,
    verifyMfa: unsupported,
    setupAdmin: unsupported,
    requestPasswordReset: unsupported,
    resetPassword: unsupported,
    changePassword: unsupported,
    verifyCode: unsupported,
    requestEmailVerification: unsupported,
    verifyEmail: unsupported,
    getMfaStatus: unsupported,
    setupTotp: unsupported,
    confirmTotp: unsupported,
    disableMfa: unsupported,
    regenerateRecoveryCodes: unsupported,
  };
  // The underlying service is a narrow local stand-in; unsupported account operations fail explicitly.
  const auth = new Proxy(result, {
    get(target, key) {
      if (key === 'state')
        return () => ({
          isAuthenticated: target.isAuthenticated,
          isLoading: false,
          userId: target.userId,
          isAdmin: false,
        });
      if (key === 'subscribe') return backend.subscribe;
      return Reflect.get(target, key) ?? unsupported;
    },
  }) as unknown as RxFerrixAuth;
  return {
    ...result,
    get isAuthenticated() {
      return result.isAuthenticated;
    },
    get userId() {
      return result.userId;
    },
    auth,
  };
}
export function useCurrentUser(options: { client?: RxFerrixClient } = {}): {
  readonly data: User | null | undefined;
  readonly isLoading: boolean;
} {
  const auth = useAuth(options);
  return {
    get data() {
      observe();
      return auth.isAuthenticated
        ? {
            _id: 'you' as Id<'_users'>,
            _creationTime: 0,
            name: backend.state.people[0].name,
            email: 'alex@fieldwork.design',
            emailVerified: true,
            image: null,
          }
        : null;
    },
    get isLoading() {
      return false;
    },
  };
}
export async function uploadFile(
  _url: string,
  file: Blob,
  options?: UploadFileOptions,
): Promise<Id<'_storage'>> {
  if (options?.signal?.aborted) throw new DOMException('Upload cancelled', 'AbortError');
  const id = crypto.randomUUID() as Id<'_storage'>;
  backend.state.attachments.push({
    id,
    name: options?.filename ?? (file instanceof File ? file.name : 'Attachment'),
    url: URL.createObjectURL(file),
    contentType: options?.contentType ?? file.type,
    size: file.size,
  });
  options?.onProgress?.({ loaded: file.size, total: file.size });
  return id;
}
/** Mount once in the root. Timers stop on unmount and pause in background tabs. */
export function startDemoActivity() {
  let beat = 0;
  const timer = setInterval(() => {
    if (document.hidden || !backend.authenticated) return;
    beat++;
    backend.typing = beat % 4 === 1 ? [{ room: 'design', user: 'sofia', name: 'Sofia' }] : [];
    if (beat % 12 === 0) {
      const author = backend.state.people[beat % 2 ? 2 : 3];
      const m: MessageView = {
        _id: crypto.randomUUID(),
        room: 'design',
        author,
        body: [
          'Just added a few little details to the prototype. ✨',
          'Taking a quick coffee break — back in a bit! ☕',
          'The new direction is feeling so good.',
        ][(beat / 12) % 3],
        _creationTime: Date.now(),
        replyCount: 0,
        attachments: [],
        reactions: [],
        deleted: false,
      };
      backend.state.messages.push(m);
      backend.state.rooms.find((r) => r._id === 'design')!.unread++;
    }
    if (beat % 6 === 0) {
      const r = backend.state.rooms.find((r) => r._id === 'build')!;
      r.voice = r.voice.length
        ? []
        : [
            {
              user: 'jules',
              name: 'Jules Laurent',
              color: '#d1b674',
              muted: false,
              deafened: false,
              sharingScreen: false,
              video: false,
            },
          ];
    }
    backend.notify();
  }, 4000);
  return () => clearInterval(timer);
}
export function setupMock() {
  backend.authenticated = localStorage.getItem('pingo-session') !== 'signed-out';
}
