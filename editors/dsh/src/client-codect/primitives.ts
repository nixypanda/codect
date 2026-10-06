/**
 * primitives.ts — Guarded seam to the DSH shared UI primitives.
 *
 * The client bundle marks `@deepseek-ai/dsh-client-ui-primitives` external and
 * resolves it against the platform module table at runtime. A missing or
 * mismatched table entry must degrade to the fallback markup, never crash
 * activation, so the require is guarded and every export may be `undefined`.
 */

declare function require(id: string): any;

const api: any = (() => {
  try {
    return require("@deepseek-ai/dsh-client-ui-primitives") || {};
  } catch {
    return {};
  }
})();

export const ReadBlock: any = api.ReadBlock;
export const DiffBlock: any = api.DiffBlock;
export const SegmentedControl: any = api.SegmentedControl;
export const FileTypeIcon: any = api.FileTypeIcon;
export const classifyFileType: any = api.classifyFileType;
export const IconFolderOpenRegular: any = api.IconFolderOpenRegular;
export const IconFolderCloseRegular: any = api.IconFolderCloseRegular;
