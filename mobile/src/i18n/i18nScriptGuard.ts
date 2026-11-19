import { readFileSync } from 'node:fs';

const dictionary = (name: string) => readFileSync(new URL(`./opencc/${name}`, import.meta.url), 'utf8');
const rows = (data: string) => data.split('\n').filter(line => line && !line.startsWith('#')).map(line => line.split('\t'));
export const exclusiveCharacters = (name: string) => new Set(rows(dictionary(name))
  .filter(([source, targets]) => !targets!.split(' ').includes(source!)).map(([source]) => source!));

const simplified = exclusiveCharacters('STCharacters.txt');
const traditional = exclusiveCharacters('TSCharacters.txt');
// STCharacters is standard Traditional; zh-TW also permits OpenCC's Taiwan
// target forms (e.g. 羣 → 群). Keep all other script checks intact.
for (const [, targets] of rows(dictionary('TWVariants.txt'))) {
  for (const character of targets!.split(' ')) simplified.delete(character);
}

export function wrongScriptCharacters(language: 'zh-CN' | 'zh-TW', text: string): string[] {
  const forbidden = language === 'zh-TW' ? simplified : traditional;
  return [...new Set([...text].filter(character => forbidden.has(character)))];
}
