import { Marked, Renderer } from 'marked';
import DOMPurify from 'dompurify';
const renderer = new Renderer();
renderer.text = function (token) {
  const text = Renderer.prototype.text.call(this, token);
  return text.replace(/(^|\s)@([\p{L}\p{N}_-]+)/gu, '$1<span class="mention">@$2</span>');
};
const markdown = new Marked({ renderer, breaks: true });
/** Sanitize the final HTML, after every markdown extension has run. */
export function renderMarkdown(text: string): string {
  return DOMPurify.sanitize(markdown.parse(text, { async: false }), {
    FORBID_TAGS: ['style', 'iframe'],
    FORBID_ATTR: ['style'],
  });
}
