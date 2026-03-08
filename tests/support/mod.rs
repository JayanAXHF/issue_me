use ratatui::buffer::Buffer;

pub fn buffer_to_string(buf: &Buffer) -> String {
    let mut lines = Vec::new();

    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            #[allow(deprecated)]
            let cell = buf.get(x, y);
            line.push_str(cell.symbol());
        }
        lines.push(line);
    }

    while let Some(last) = lines.last() {
        if last.trim().is_empty() {
            lines.pop();
        } else {
            break;
        }
    }

    lines.join("\n")
}
