//! Markdown → blocks (BRIEF §9.2): headings become their own paragraphs, link text is kept and URLs dropped, code blocks
//! are skipped, list items become sentences, tables are read row by row, images are dropped.

use super::Block;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub fn markdown_to_blocks(md: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut blocks = Vec::new();
    let mut cur = String::new();
    let mut heading = false;
    let mut in_code = 0usize;
    let mut in_image = 0usize;
    let mut in_item = 0usize;
    let mut table_row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_table = false;

    let flush =
        |cur: &mut String, heading: &mut bool, blocks: &mut Vec<Block>, as_sentence: bool| {
            let mut t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() {
                if as_sentence && !t.ends_with(['.', '!', '?', ':', ';']) {
                    t.push('.');
                }
                blocks.push(Block {
                    text: t,
                    heading: *heading,
                });
            }
            cur.clear();
            *heading = false;
        };

    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut cur, &mut heading, &mut blocks, false);
                heading = true;
            }
            Event::End(TagEnd::Heading(_)) => flush(&mut cur, &mut heading, &mut blocks, true),
            Event::Start(Tag::CodeBlock(_)) => in_code += 1,
            Event::End(TagEnd::CodeBlock) => in_code = in_code.saturating_sub(1),
            Event::Start(Tag::Image { .. }) => in_image += 1,
            Event::End(TagEnd::Image) => in_image = in_image.saturating_sub(1),
            Event::Start(Tag::Item) => {
                flush(&mut cur, &mut heading, &mut blocks, false);
                in_item += 1;
            }
            Event::End(TagEnd::Item) => {
                flush(&mut cur, &mut heading, &mut blocks, true);
                in_item = in_item.saturating_sub(1);
            }
            Event::Start(Tag::Paragraph) | Event::Start(Tag::BlockQuote(_)) => {
                if in_item == 0 {
                    flush(&mut cur, &mut heading, &mut blocks, false);
                }
            }
            Event::End(TagEnd::Paragraph) | Event::End(TagEnd::BlockQuote(_)) => {
                if in_item == 0 {
                    flush(&mut cur, &mut heading, &mut blocks, false);
                }
            }
            Event::Start(Tag::Table(_)) => in_table = true,
            Event::End(TagEnd::Table) => in_table = false,
            Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => table_row.clear(),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                let t = table_row
                    .iter()
                    .filter(|c| !c.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !t.is_empty() {
                    blocks.push(Block::para(format!("{t}.")));
                }
                table_row.clear();
            }
            Event::End(TagEnd::TableCell) => {
                table_row.push(cell.split_whitespace().collect::<Vec<_>>().join(" "));
                cell.clear();
            }
            Event::Text(t) | Event::Code(t) if in_code == 0 && in_image == 0 => {
                if in_table {
                    cell.push_str(&t);
                } else {
                    cur.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak if in_code == 0 => {
                if in_table {
                    cell.push(' ');
                } else {
                    cur.push(' ');
                }
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::Rule => {}
            _ => {}
        }
    }
    flush(&mut cur, &mut heading, &mut blocks, false);
    blocks
}
