import { readFile } from "fs/promises";

const MAX = 3;

async function load(path) {
  const text = await readFile(path, "utf8");
  return text.split("\n").slice(0, MAX);
}

load("a.txt").then((lines) => console.log(lines));
