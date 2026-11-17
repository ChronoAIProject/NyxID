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
