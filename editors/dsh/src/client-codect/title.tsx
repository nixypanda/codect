/**
 * title.tsx — Tab title chip for the codect sidebar tabs.
 * Pattern follows dsh-client-ui-sidebar-files FilesTitle.
 */

import React from "react";
import { SHOW_KIND, DIFF_KIND } from "./definition";
import { FileTypeIcon } from "./primitives";

export interface CodectTitleProps {
  kind: typeof SHOW_KIND | typeof DIFF_KIND;
  revision?: string;
  base?: string;
  target?: string;
}

export function CodectTitle({ kind, revision, base, target }: CodectTitleProps) {
  if (kind === SHOW_KIND) {
    return (
      <span className="codect-title">
        <span className="codect-title-icon">
          {typeof FileTypeIcon === "function" ? (
            <FileTypeIcon kind="folder" size={16} />
          ) : (
            "◫"
          )}
        </span>
        <span className="codect-title-text">Codect</span>
        {revision && <span className="codect-title-rev">{revision}</span>}
      </span>
    );
  }
  if (kind === DIFF_KIND) {
    return (
      <span className="codect-title">
        <span className="codect-title-icon">⇄</span>
        <span className="codect-title-text">Diff</span>
        {base && target && (
          <span className="codect-title-rev">{base} → {target}</span>
        )}
      </span>
    );
  }
  return <span className="codect-title">Codect</span>;
}
