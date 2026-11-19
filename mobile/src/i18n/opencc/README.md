# OpenCC character dictionaries

STCharacters.txt, TSCharacters.txt and LICENSE are unmodified from OpenCC 1.1.9:
https://github.com/BYVoid/OpenCC/tree/ver.1.1.9/data/dictionary

The catalog test reads each direction independently. A source character is
exclusive to that script only when none of its mapped targets equals itself.
Mappings with multiple targets must remain: 发 → 發/髮 is still Simplified-only.
Self-inclusive mappings such as 着 → 着/著 remain allowed in either catalog.
Characters absent from a dictionary, such as 刷、窗、缺, remain allowed as well.

The files are vendored so tests require no network access. See LICENSE for the
upstream Apache-2.0 license.

TWVariants.txt is unmodified from OpenCC ver.1.1.9, pinned commit
`556ed22496d650bd0b13b6c163be9814637970ae`:
https://github.com/BYVoid/OpenCC/blob/556ed22496d650bd0b13b6c163be9814637970ae/data/dictionary/TWVariants.txt

SHA-256: `30e6f8395edbfdd74e293fd8b9c62105d787c849fbb208d2a7832eac696734d7`.
It is covered by the same Apache-2.0 LICENSE. It is read as data only.
The zh-TW guard permits its Taiwan target forms in addition to standard
Traditional characters. For example, 羣 → 群 makes 社群 valid Taiwan copy;
Simplified 区 and 门 remain rejected.
