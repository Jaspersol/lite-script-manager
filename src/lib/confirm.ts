import { writable } from "svelte/store";

export type ConfirmOptions = {
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
};

export type ConfirmRequest = Required<ConfirmOptions> & { resolve: (ok: boolean) => void };

export const confirmRequest = writable<ConfirmRequest | null>(null);

/** In-app replacement for `window.confirm`, which the WebView does not show reliably. */
export function confirmDialog(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    confirmRequest.update((previous) => {
      previous?.resolve(false);
      return {
        confirmLabel: "确定",
        danger: false,
        ...options,
        resolve,
      };
    });
  });
}
