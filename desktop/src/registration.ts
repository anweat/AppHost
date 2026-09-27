export interface Issue { field: string; message: string }
export interface Candidate {
  path: string; human_label: string; selected: boolean;
  manifest: { services: { name: string; command?: string[]; [key: string]: unknown }[]; [key: string]: unknown } | null;
  issues: Issue[]; notes: Issue[]; [key: string]: unknown;
}
export interface Draft {
  root: string; candidates: Candidate[]; issues: Issue[];
  groups: { members: string[]; candidate: Candidate }[];
  [key: string]: unknown;
}
export interface ManifestRead { revision: number; base_sha256: string; text: string; manifest_path: string }
export interface ManifestReview extends Omit<ManifestRead, 'text'> {
  before_text: string; after_text: string; launch_sha256: string;
  previous_confirmation: { manifest: unknown; services: unknown };
  manifest: unknown; warnings: Issue[];
}
export interface SaveReceipt { confirmed?: boolean; operation_id?: string; operation_done?: boolean; error?: {code: string; message: string} }

export interface ScanResult { token: string; draft: Draft; editable: boolean[] }
