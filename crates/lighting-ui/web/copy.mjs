import { copyFileSync } from "node:fs";
for (const f of ["index.html", "app.css", "icon.svg"]) copyFileSync(`static/${f}`, `dist/${f}`);
