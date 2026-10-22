import { invoke } from "@tauri-apps/api/core";

export interface Snippet { id: string; name: string; script: string; tags: string[]; createdAt: number | null; updatedAt: number | null }
export interface SnippetDraft { id: string; name: string; script: string; tags: string }
export const listSnippets = (query = "") => invoke<Snippet[]>("list_snippets", { query });
export const saveSnippet = (draft: SnippetDraft) => invoke<string>("save_snippet", { draft });
export const deleteSnippet = (id: string) => invoke<void>("delete_snippet", { id });
