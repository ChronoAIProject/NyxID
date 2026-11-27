import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const root = new URL('../../', import.meta.url);
test('P1-6 all committed iOS permission rationales have matching Expo and native Chinese localizations', () => {
  const plist = readFileSync(new URL('ios/NyxIDMobile/Info.plist', root), 'utf8');
  const keys = [...plist.matchAll(/<key>(NS\w+UsageDescription)<\/key>/g)].map(m=>m[1]!);
  const config = readFileSync(new URL('app.config.ts', root), 'utf8');
  const project = readFileSync(new URL('ios/NyxIDMobile.xcodeproj/project.pbxproj', root), 'utf8');
  for (const language of ['zh-Hans','zh-Hant']) {
    assert.ok(config.includes(`"${language}": "./locales/${language}.json"`));
    const { ios: values } = JSON.parse(readFileSync(new URL(`locales/${language}.json`, root), 'utf8'));
    const native = readFileSync(new URL(`ios/NyxIDMobile/Supporting/${language}.lproj/InfoPlist.strings`, root), 'utf8');
    assert.ok(project.includes(`${language}.lproj`));
    for (const key of keys) {
      assert.match(values[key], /[\u3400-\u9fff]/, `${language} ${key}`);
      assert.ok(native.includes(`${key} = "${values[key]}";`));
    }
  }
});
