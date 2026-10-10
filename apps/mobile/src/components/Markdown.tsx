import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Render agent markdown (summaries, results) to React elements. Raw HTML is
 * never rendered — react-markdown escapes it — and links are constrained to
 * http(s) and opened outside the app, so displayed content cannot navigate the
 * UI or run script.
 */
export function Markdown({ children }: { children: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children: label }) => (
            <a
              href={href}
              rel="noreferrer noopener"
              target="_blank"
              onClick={(event) => {
                event.preventDefault();
                if (href && /^https?:\/\//i.test(href)) {
                  window.open(href, "_blank", "noopener,noreferrer");
                }
              }}
            >
              {label}
            </a>
          ),
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
