import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../api";

/**
 * Render trusted-ish markdown (an agent's result or reviewer report) to React
 * elements. Raw HTML is never rendered — react-markdown escapes it — and links
 * open in the browser instead of navigating the app away.
 */
export default function Markdown({ children }: { children: string }) {
  return (
    <article className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(e) => {
                e.preventDefault();
                if (href && /^https?:\/\//i.test(href)) {
                  void api.openExternal(href).catch(() => {});
                }
              }}
            >
              {children}
            </a>
          ),
        }}
      >
        {children}
      </ReactMarkdown>
    </article>
  );
}
