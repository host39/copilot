import { inlineImage, safeLink, transformUrl } from "@web/components/Chat/urls";
import { type ReactNode, useState } from "react";
import { LuCheck, LuCopy } from "react-icons/lu";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

type Node = { type: string; value?: string; children?: Node[] };

/** The text of a syntax tree node (a code block's source). */
export function nodeText(node: Node | undefined): string {
  if (!node) return "";
  if (typeof node.value === "string") return node.value;
  return (node.children ?? []).map(nodeText).join("");
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1_500);
        });
      }}
      aria-label={copied ? "Copied" : "Copy code"}
      title={copied ? "Copied" : "Copy code"}
      className="absolute top-1.5 right-1.5 flex size-8 items-center justify-center rounded-md bg-canvas/80 text-muted hover:text-fg"
    >
      {copied ? <LuCheck aria-hidden /> : <LuCopy aria-hidden />}
    </button>
  );
}

const components: Components = {
  a({ node, href, children }) {
    const target = href ? safeLink(href) : null;
    if (!target) return <span>{children}</span>;
    const label = nodeText(node as Node | undefined).trim();
    const shown = safeLink(label) === target || label === target.replace(/^mailto:/, "");
    return (
      <>
        <a href={target} target="_blank" rel="noopener noreferrer" className="text-link">
          {children}
        </a>
        {shown ? null : <span className="ml-1 break-all text-xs text-muted">{`(${target})`}</span>}
      </>
    );
  },
  img({ src, alt }) {
    const source = typeof src === "string" ? src : "";
    if (inlineImage(source)) return <img src={source} alt={alt ?? ""} className="max-w-full" />;
    const target = safeLink(source);
    if (!target) return <span>{alt ? `[${alt}]` : "[image]"}</span>;
    return (
      <a href={target} target="_blank" rel="noopener noreferrer" className="text-link break-all">
        {target}
      </a>
    );
  },
  pre({ node, children }) {
    return (
      <div className="relative">
        <pre className="overflow-x-auto rounded-lg bg-raised p-3 pr-11 text-sm">{children}</pre>
        <CopyButton text={nodeText(node as Node | undefined)} />
      </div>
    );
  },
};

/**
 * Model output as Markdown (GitHub flavored), never as HTML: raw HTML shows as text, links open
 * in a new tab with their URL beside them, only safe protocols are kept, and code blocks have a
 * copy button.
 */
export function Markdown({ children }: { children: string }): ReactNode {
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={transformUrl}
        components={components}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
