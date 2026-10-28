use std::fs::File;
use std::io::{self, Read, Write};

fn main() -> io::Result<()> {
    let mut file = File::open("docs/api.md")?;
    let mut markdown = String::new();
    file.read_to_string(&mut markdown)?;

    let (toc_html, content_html) = parse_markdown(&markdown);

    let full_html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Koburi - Documentation</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Fira+Code:wght@400;500&family=Inter:wght@300;400;500;600;700&display=swap" rel="stylesheet">
    <script>
        const savedTheme = localStorage.getItem('theme') || 'light';
        document.documentElement.setAttribute('data-theme', savedTheme);
    </script>
    <style>
        :root {{
            --bg-main: #f8fafc;
            --bg-card: #ffffff;
            --bg-sidebar: #f1f5f9;
            --border-color: #e2e8f0;
            
            --text-primary: #0f172a;
            --text-secondary: #334155;
            --text-muted: #64748b;
            
            --accent: #4f46e5;
            --accent-hover: #4338ca;
            --accent-light: rgba(79, 70, 229, 0.05);
            
            --code-bg: #f1f5f9;
            --code-text: #0f172a;
            
            --code-block-bg: #f8fafc;
            --code-block-header: #f1f5f9;
            --code-block-text: #0f172a;
            
            --font-sans: 'Inter', system-ui, -apple-system, sans-serif;
            --font-mono: 'Fira Code', monospace;
            
            --max-width: 840px;
            --sidebar-width: 280px;
        }}

        [data-theme="dark"] {{
            --bg-main: #0b0f19;
            --bg-card: #111827;
            --bg-sidebar: #0f172a;
            --border-color: #1f2937;
            
            --text-primary: #f9fafb;
            --text-secondary: #cbd5e1;
            --text-muted: #64748b;
            
            --accent: #6366f1;
            --accent-hover: #818cf8;
            --accent-light: rgba(99, 102, 241, 0.1);
            
            --code-bg: #1e293b;
            --code-text: #38bdf8;
            
            --code-block-bg: #111827;
            --code-block-header: rgba(30, 41, 59, 0.5);
            --code-block-text: #cbd5e1;
        }}

        * {{
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }}

        html {{
            scroll-behavior: smooth;
        }}

        body {{
            background-color: var(--bg-main);
            color: var(--text-secondary);
            font-family: var(--font-sans);
            line-height: 1.6;
            display: flex;
            min-height: 100vh;
            transition: background-color 0.3s ease, color 0.3s ease;
        }}

        .sidebar {{
            width: var(--sidebar-width);
            background-color: var(--bg-sidebar);
            border-right: 1px solid var(--border-color);
            position: fixed;
            top: 0;
            bottom: 0;
            left: 0;
            overflow-y: auto;
            padding: 2.5rem 1.5rem;
            display: flex;
            flex-direction: column;
            transition: background-color 0.3s ease, border-color 0.3s ease;
        }}

        .sidebar-header {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
            margin-bottom: 2.5rem;
        }}

        .logo-icon {{
            font-size: 1.5rem;
        }}

        .logo-text {{
            font-weight: 700;
            font-size: 1.25rem;
            color: var(--text-primary);
            letter-spacing: -0.025em;
        }}

        .theme-toggle-btn {{
            background: none;
            border: 1px solid var(--border-color);
            color: var(--text-muted);
            cursor: pointer;
            padding: 0.4rem;
            border-radius: 8px;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: all 0.2s ease;
            margin-left: auto;
        }}

        .theme-toggle-btn:hover {{
            color: var(--text-primary);
            background-color: var(--border-color);
            border-color: var(--text-muted);
        }}

        .sun-icon {{
            display: none;
        }}
        .moon-icon {{
            display: block;
        }}

        [data-theme="dark"] .sun-icon {{
            display: block;
        }}
        [data-theme="dark"] .moon-icon {{
            display: none;
        }}

        .toc {{
            list-style: none;
        }}

        .toc li {{
            margin-bottom: 0.5rem;
        }}

        .toc a {{
            color: var(--text-muted);
            text-decoration: none;
            font-size: 0.9rem;
            transition: all 0.2s ease;
            display: block;
            padding: 0.35rem 0.75rem;
            border-radius: 6px;
            font-weight: 500;
        }}

        .toc a:hover {{
            color: var(--text-primary);
            background-color: var(--accent-light);
        }}

        .toc-h3 {{
            margin-left: 1.25rem;
        }}

        .toc-h3 a {{
            font-size: 0.825rem;
            font-weight: 400;
        }}

        .main-wrapper {{
            margin-left: var(--sidebar-width);
            flex: 1;
            display: flex;
            justify-content: center;
            padding: 3rem 2.5rem;
            width: calc(100% - var(--sidebar-width));
            transition: background-color 0.3s ease;
        }}

        .content {{
            max-width: var(--max-width);
            width: 100%;
        }}

        h2 {{
            color: var(--text-primary);
            font-size: 1.75rem;
            font-weight: 700;
            margin-top: 3.5rem;
            margin-bottom: 1.25rem;
            border-bottom: 1px solid var(--border-color);
            padding-bottom: 0.5rem;
            letter-spacing: -0.02em;
            transition: border-color 0.3s ease, color 0.3s ease;
        }}

        h3 {{
            color: var(--text-primary);
            font-size: 1.35rem;
            font-weight: 600;
            margin-top: 2.25rem;
            margin-bottom: 0.75rem;
            letter-spacing: -0.01em;
            transition: color 0.3s ease;
        }}

        p {{
            margin-bottom: 1.25rem;
            font-size: 1rem;
            color: var(--text-secondary);
            transition: color 0.3s ease;
        }}

        a {{
            color: var(--accent);
            text-decoration: none;
            transition: color 0.15s ease;
        }}

        a:hover {{
            color: var(--accent-hover);
            text-decoration: underline;
        }}

        code {{
            font-family: var(--font-mono);
            font-size: 0.85rem;
            background-color: var(--code-bg);
            color: var(--code-text);
            padding: 0.125rem 0.375rem;
            border-radius: 4px;
            transition: background-color 0.3s ease, color 0.3s ease;
        }}

        h2 code, h3 code {{
            font-size: inherit;
            background: none;
            padding: 0;
            color: inherit;
        }}

        .code-container {{
            background-color: var(--code-block-bg);
            border: 1px solid var(--border-color);
            border-radius: 12px;
            margin: 1.75rem 0;
            overflow: hidden;
            transition: background-color 0.3s ease, border-color 0.3s ease;
        }}

        .code-header {{
            background-color: var(--code-block-header);
            border-bottom: 1px solid var(--border-color);
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 0.6rem 1.25rem;
            transition: background-color 0.3s ease, border-color 0.3s ease;
        }}

        .code-lang {{
            font-size: 0.75rem;
            font-weight: 700;
            text-transform: uppercase;
            color: var(--text-muted);
            letter-spacing: 0.05em;
        }}

        .copy-btn {{
            background-color: transparent;
            border: 1px solid var(--border-color);
            color: var(--text-secondary);
            padding: 0.3rem 0.75rem;
            border-radius: 6px;
            font-size: 0.75rem;
            cursor: pointer;
            transition: all 0.15s ease;
            font-weight: 500;
        }}

        .copy-btn:hover {{
            background-color: var(--border-color);
            color: var(--text-primary);
            border-color: var(--text-muted);
        }}

        pre {{
            padding: 1.25rem;
            overflow-x: auto;
            margin: 0;
        }}

        pre code {{
            background-color: transparent;
            color: var(--code-block-text);
            padding: 0;
            border-radius: 0;
            font-size: 0.875rem;
            display: block;
            line-height: 1.6;
            transition: color 0.3s ease;
        }}

        ul {{
            margin-left: 1.5rem;
            margin-bottom: 1.5rem;
        }}

        li {{
            margin-bottom: 0.5rem;
            color: var(--text-secondary);
            transition: color 0.3s ease;
        }}

        li code {{
            font-size: 0.825rem;
        }}

        @media (max-width: 768px) {{
            body {{
                flex-direction: column;
            }}
            
            .sidebar {{
                position: relative;
                width: 100%;
                border-right: none;
                border-bottom: 1px solid var(--border-color);
                padding: 1.5rem;
            }}
            
            .main-wrapper {{
                margin-left: 0;
                width: 100%;
                padding: 2rem 1.25rem;
            }}
        }}
    </style>
</head>
<body>
    <nav class="sidebar">
        <div class="sidebar-header">
            <span class="logo-icon">🦫</span>
            <span class="logo-text">Koburi Docs</span>
            <button id="theme-toggle" class="theme-toggle-btn" aria-label="Toggle theme">
                <svg class="moon-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
                <svg class="sun-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/></svg>
            </button>
        </div>
        <ul class="toc">
            {toc}
        </ul>
    </nav>
    <div class="main-wrapper">
        <main class="content">
            {content}
        </main>
    </div>

    <script>
    const themeToggleBtn = document.getElementById('theme-toggle');
    themeToggleBtn.addEventListener('click', () => {{
        const currentTheme = document.documentElement.getAttribute('data-theme');
        const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
        document.documentElement.setAttribute('data-theme', newTheme);
        localStorage.setItem('theme', newTheme);
    }});

    function copyCode(button) {{
        const container = button.closest('.code-container');
        const codeBlock = container.querySelector('pre code');
        const text = codeBlock.innerText;
        
        navigator.clipboard.writeText(text).then(() => {{
            button.innerText = 'Copied!';
            button.style.borderColor = '#10b981';
            button.style.color = '#10b981';
            setTimeout(() => {{
                button.innerText = 'Copy';
                button.style.borderColor = '';
                button.style.color = '';
            }}, 2000);
        }}).catch(err => {{
            console.error('Failed to copy text: ', err);
        }});
    }}
    </script>
</body>
</html>"#,
        toc = toc_html,
        content = content_html
    );

    let mut out = File::create("api.html")?;
    out.write_all(full_html.as_bytes())?;
    println!("Successfully generated api.html");
    Ok(())
}

fn parse_markdown(md: &str) -> (String, String) {
    let mut toc = String::new();
    let mut content = String::new();

    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_block = String::new();

    let mut in_list = false;

    let lines: Vec<&str> = md.lines().collect();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") && !in_code {
            let title = &trimmed[3..];
            let clean_title = clean_inline_formatting(title);
            let slug = slugify(&clean_title);
            toc.push_str(&format!(
                r##"<li class="toc-h2"><a href="#{}">{}</a></li>{}"##,
                slug, clean_title, "\n"
            ));
        } else if trimmed.starts_with("### ") && !in_code {
            let title = &trimmed[4..];
            let clean_title = clean_inline_formatting(title);
            let slug = slugify(&clean_title);
            toc.push_str(&format!(
                r##"<li class="toc-h3"><a href="#{}">{}</a></li>{}"##,
                slug, clean_title, "\n"
            ));
        }
    }

    for line in lines {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_code {
                content.push_str(&format!(
                    r#"<div class="code-container">
<div class="code-header">
  <span class="code-lang">{}</span>
  <button class="copy-btn" onclick="copyCode(this)">Copy</button>
</div>
<pre><code class="language-{}">{}</code></pre>
</div>{}"#,
                    code_lang.to_uppercase(),
                    code_lang,
                    html_escape(&code_block),
                    "\n"
                ));
                code_block.clear();
                code_lang.clear();
                in_code = false;
            } else {
                in_code = true;
                code_lang = trimmed[3..].to_string();
                if code_lang.is_empty() {
                    code_lang = "text".to_string();
                }
            }
            continue;
        }

        if in_code {
            code_block.push_str(&line);
            code_block.push('\n');
            continue;
        }

        if trimmed.starts_with("## ") {
            if in_list {
                content.push_str("</ul>\n");
                in_list = false;
            }
            let title = &trimmed[3..];
            let parsed_title = replace_inline(title);
            let slug = slugify(&clean_inline_formatting(title));
            content.push_str(&format!(
                r#"<h2 id="{}">{}</h2>{}"#,
                slug, parsed_title, "\n"
            ));
            continue;
        }
        if trimmed.starts_with("### ") {
            if in_list {
                content.push_str("</ul>\n");
                in_list = false;
            }
            let title = &trimmed[4..];
            let parsed_title = replace_inline(title);
            let slug = slugify(&clean_inline_formatting(title));
            content.push_str(&format!(
                r#"<h3 id="{}">{}</h3>{}"#,
                slug, parsed_title, "\n"
            ));
            continue;
        }

        if trimmed.starts_with("- ") {
            if !in_list {
                content.push_str("<ul>\n");
                in_list = true;
            }
            let item_text = &trimmed[2..];
            let parsed_item = replace_inline(item_text);
            content.push_str(&format!("<li>{}</li>\n", parsed_item));
            continue;
        }

        if in_list && !trimmed.is_empty() {
            content.push_str("</ul>\n");
            in_list = false;
        }

        if trimmed.is_empty() {
            if in_list {
                content.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }

        let parsed_para = replace_inline(line);
        content.push_str(&format!("<p>{}</p>\n", parsed_para));
    }

    if in_list {
        content.push_str("</ul>\n");
    }

    (toc, content)
}

fn html_escape(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars() {
        match c {
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '&' => result.push_str("&amp;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
}

fn replace_inline(text: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut in_code = false;
    let mut current_segment = String::new();

    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            if in_code {
                result.push_str("<code>");
                result.push_str(&html_escape(&current_segment));
                result.push_str("</code>");
                current_segment.clear();
                in_code = false;
            } else {
                in_code = true;
            }
            i += 1;
        } else if !in_code && chars[i] == '[' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() && j + 1 < chars.len() && chars[j + 1] == '(' {
                let mut k = j + 2;
                while k < chars.len() && chars[k] != ')' {
                    k += 1;
                }
                if k < chars.len() {
                    let link_text: String = chars[i + 1..j].iter().collect();
                    let link_url: String = chars[j + 2..k].iter().collect();
                    result.push_str(&format!(
                        "<a href=\"{}\">{}</a>",
                        link_url,
                        replace_inline(&link_text)
                    ));
                    i = k + 1;
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        } else {
            if in_code {
                current_segment.push(chars[i]);
            } else {
                match chars[i] {
                    '<' => result.push_str("&lt;"),
                    '>' => result.push_str("&gt;"),
                    '&' => result.push_str("&amp;"),
                    _ => result.push(chars[i]),
                }
            }
            i += 1;
        }
    }

    if in_code {
        result.push('`');
        result.push_str(&current_segment);
    }
    result
}

fn clean_inline_formatting(text: &str) -> String {
    let mut cleaned = text.replace("`", "");
    while let Some(start) = cleaned.find('[') {
        if let Some(mid) = cleaned[start..].find(']') {
            let mid_idx = start + mid;
            if cleaned[mid_idx..].starts_with("](") {
                if let Some(end) = cleaned[mid_idx..].find(')') {
                    let end_idx = mid_idx + end;
                    let link_text = cleaned[start + 1..mid_idx].to_string();
                    cleaned.replace_range(start..=end_idx, &link_text);
                    continue;
                }
            }
        }
        break;
    }
    cleaned
}

fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c)
            } else if c.is_whitespace() || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
