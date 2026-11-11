/**
 * The Web Lock a worker takes before it opens an OPFS store: one holder per store path, so a
 * second tab or worker gets `StoreLocked` instead of hanging or corrupting the file. Web Locks
 * are released when the holding worker terminates.
 */

import { VertepError } from "./errors.js";

/** Releases a held store lock; resolves once another holder can take it. */
export type Release = () => Promise<void>;

/** The lock name of a store path. */
export function lockName(path: string): string {
  return `vertep:store:${path}`;
}

/**
 * Takes the lock of `path` if nobody holds it. Rejects with `StoreLocked` when another holder
 * has it, and with `StorageUnavailable` when the environment has no Web Locks.
 */
// @tg: implements:: [[openspec:wasm-bindings#Browser Journal Storage]] {confidence: partial, note: "Web Lock exclusivity per store path"}
export function takeStoreLock(path: string): Promise<Release> {
  const locks = (globalThis as { navigator?: { locks?: LockManagerLike } }).navigator?.locks;
  if (!locks) {
    return Promise.reject(
      new VertepError("StorageUnavailable", "Web Locks are unavailable, so the store cannot be held exclusively"),
    );
  }
  return new Promise<Release>((resolve, reject) => {
    const held: Promise<unknown> = locks
      .request(lockName(path), { ifAvailable: true }, (lock: unknown) => {
        if (!lock) {
          reject(new VertepError("StoreLocked", `the store ${path} is open in another worker or tab`));
          return undefined;
        }
        return new Promise<void>((release) =>
          resolve(() => {
            release();
            return held.then(() => undefined);
          }),
        );
      });
    held.catch(reject);
  });
}

interface LockManagerLike {
  request(
    name: string,
    options: { ifAvailable: boolean },
    callback: (lock: unknown) => Promise<void> | undefined,
  ): Promise<unknown>;
}
