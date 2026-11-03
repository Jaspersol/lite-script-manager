import { writable } from "svelte/store";

export type PageAction = {
  label: string;
  icon: "plus" | "trash" | "sliders";
  kind?: "primary" | "danger";
  disabled?: boolean;
  run: () => void;
};

export const pageAction = writable<PageAction | null>(null);
