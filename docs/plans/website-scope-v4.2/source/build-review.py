"""Build an image-only comparison board; no embedded site, scripts or animation."""
from pathlib import Path
from html import escape

root = Path(__file__).resolve().parent.parent
items = [
    ('C01', 'Blog 先静态发布', 'Publish reviewed static articles first', '保留', 'blog',
     'v4.1 已有精选、分类、文章详情和相关阅读，但没有明确本期的正式内容发布方式。',
     '顶部及移动菜单的 Blog 入口默认隐藏，保留独立恢复开关；Footer、直接访问和 SEO/AI 规则不改。页面阅读体验保持；本期维护经审核的静态文章，通过 PR 发布，CMS 成熟后再接。下面两张首屏图应基本相同。',
     '先交付可阅读内容，避免把 CMS 建设变成官网前置依赖。旧 mock 草稿不能直接当正式文章发布。'),
    ('C02', '订阅本期隐藏，保留实现', 'Default-off newsletter; implementation retained', '默认隐藏', 'newsletter',
     '邮箱输入框 + 订阅按钮 + “每周一个实用点子 / 随时退订”承诺，旁边有 Discord。',
     '默认发布输出不包含订阅控件、承诺或空框，只展示 Discord；保留表单和真实 adapter，通过独立开关开放。没有“订阅即将开放”的占位。',
     '不承诺尚未具备的订阅、发送和退订能力。默认关闭，不发订阅请求；后端确认/退订链路验收后，配置并重建即可开放。'),
    ('C03', 'Contact 只留真实联系渠道', 'Keep email and Discord on Contact', '暂缓', 'contact',
     '左侧邮件表单，右侧 NyxBot 演示问答、快捷问题、输入框与 Discord。',
     '聊天卡本期默认隐藏，Discord 顶端对齐；保留组件和公共服务 adapter，独立开关控制。保留五字段表单与“定制套餐”预选，以及已经确认的邮件客户端提交方式。',
     '公共聊天服务验收、配置并重建后单项开放，不自动跟随后端上线；表单继续发往 contact@chrono-ai.fun。打开客户端不等于已发出邮件。'),
    ('C04', 'Footer 隐藏 Status', 'Hide Footer Status', '默认隐藏', 'footer',
     '法律链接行有 Status / 服务状态，原型指向首页占位。',
     'Status 默认关闭，组件与配置保留；真实 HTTPS 状态地址验收后配置开放，其余链接和默认间距不变。',
     '不把首页或健康检查接口当成面向用户的状态页。'),
    ('C06', '方案调通一个，开放一个', 'Enable each solution only after acceptance', '受控开放', 'solution',
     '9 个方案详情中的“使用此方案”都是紫色可点击链接，指向原型开始配置页。',
     '默认保留按钮文字，改为原生 disabled 灰色按钮，旁边显示“暂未开放”。一个方案通过完整验证后，只启用这一项。',
     '方案卡、详情、筛选和全局“开始使用”继续可用。配置上下文跨登录保留并完成真实任务，才算调通。'),
]

def pair(name, mobile=False):
    prefix = name + ('-mobile' if mobile else '')
    figures = []
    for variant, label in [('before', '之前 · v4.1'), ('after', '之后 · v4.2 评审稿')]:
        asset = f'assets/{prefix}-{variant}.png'
        figures.append(f'<figure><figcaption>{label}</figcaption><a href="{asset}" target="_blank" rel="noopener"><img loading="lazy" src="{asset}" alt="{escape(name)} {label}"></a></figure>')
    return '<div class="pairs' + (' mobile' if mobile else '') + '">' + ''.join(figures) + '</div>'

sections = []
for code, title, en, state, name, before, after, why in items:
    links = ' · '.join(f'<a href="previews/{name}-{lang}-{variant}.html" target="_blank" rel="noopener">{label} {lang.upper()} 切片 ↗</a>' for lang in ['zh','en'] for variant,label in [('before','v4.1'),('after','v4.2')])
    mobile = f'<details><summary>移动端对照 · 390px</summary>{pair(name,True)}</details>' if name in ['contact','solution'] else ''
    section = f'<section id="{code}"><div class="kicker">{code} <span>{state}</span></div><h2>{title}</h2><p class="en">{en}</p><div class="copy"><div><small>v4.1 · 之前</small><p>{before}</p></div><div><small>v4.2 · 本次</small><p>{after}</p></div></div>{pair(name)}<p class="why">{why}</p><p class="links">{links}</p>{mobile}</section>'
    sections.append(section)

dashboard = '''<section id="C05"><div class="kicker">C05 <span>保留</span></div><h2>默认 dashboard 逻辑保持</h2><p class="en">Keep the current default destination</p><div class="copy"><div><small>v4.1 · 已有交付约束</small><p>视觉原型没有真实登录会话；正式接入要求沿用线上认证与跳转。</p></div><div><small>v4.2 · 本次决定</small><p>继续沿用；不把这次裁剪当作调整登录落点的机会。</p></div></div><div class="flows"><div><b>已登录访问首页 /</b><span>→</span><code>/dashboard</code><em>保留</em></div><div><b>普通登录，无有效指定返回目标</b><span>→</span><code>现有默认落点</code><em>保留</em></div><div><b>有合法 return_to</b><span>→</span><code>原有校验及返回规则</code><em>保留</em></div></div><p class="why">后续再独立调整为登录后的 NyxBot 工作区。它不是官网介绍页 /nyxbot；当前产品侧是 /assistant，未来路由与方案上下文协议另行确认。</p></section>'''
sections.insert(4,dashboard)
html = '''<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>NyxID · v4.2 裁剪评审</title><style>
:root{color-scheme:dark;font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Segoe UI",sans-serif;background:#0d0d0f;color:#f7f5fa;line-height:1.7}*{box-sizing:border-box}body{margin:0}a{color:#c8afff;text-underline-offset:4px}a:hover{color:white}a:focus-visible,summary:focus-visible{outline:2px solid #b59ff9;outline-offset:4px}.wrap{max-width:1360px;padding:0 32px;margin:auto}header{padding:64px 0 38px;background:linear-gradient(130deg,#251731,#0d0d0f 70%);border-bottom:1px solid #302735}.kicker{color:#b59ff9;letter-spacing:1.5px;font-size:12px;font-weight:650}.kicker span{letter-spacing:0;background:#2c203c;margin-left:12px;padding:4px 10px;border-radius:20px}h1{font-size:clamp(32px,4vw,56px);line-height:1.2;letter-spacing:-1px;margin:18px 0}h1 span{color:#c2a1ff}h2{font-size:28px;line-height:1.35;margin:16px 0 5px}.lead{font-size:16px;max-width:930px;color:#c3bccb}.en{color:#9c91a8;font-size:13px;margin:5px 0 24px}.note{font-size:13px;color:#b7acbf;max-width:1050px}.tags{display:flex;gap:10px;flex-wrap:wrap;margin-top:24px}.tags span{font-size:12px;border:1px solid #40304e;padding:5px 12px;border-radius:20px}nav{border-bottom:1px solid #302735;background:#17131b;padding:16px 0}nav .wrap{display:flex;gap:24px;flex-wrap:wrap}nav a{font-size:13px;text-decoration:none}section{padding:46px 0;border-bottom:1px solid #302735;scroll-margin-top:20px}.copy{display:grid;grid-template-columns:1fr 1fr;gap:28px;margin:24px 0}.copy p{font-size:14px;color:#c8bfce;margin:8px 0}.copy small{font-size:12px;color:#ae97ca}.pairs{display:grid;grid-template-columns:1fr 1fr;gap:16px}figure{margin:0;border:1px solid #382d41;border-radius:12px;overflow:hidden;background:#0d0d0d}figcaption{padding:10px 14px;font-size:12px;color:#c9b9d9;background:#211927}img{display:block;width:100%;height:auto}.why{font-size:14px;color:#c9bfd2;border-left:2px solid #7f52b0;padding-left:14px;margin-top:22px}.links{font-size:12px;line-height:2}details{padding:16px;border:1px solid #3b2d48;border-radius:12px;margin-top:24px}summary{cursor:pointer;color:#c9b6dd;font-size:14px}details .pairs{margin-top:18px}.mobile{max-width:796px;margin-left:auto;margin-right:auto}.flows{border:1px solid #392d43;border-radius:12px;padding:12px 24px;background:#17121e}.flows>div{display:flex;gap:18px;align-items:center;padding:14px 0;flex-wrap:wrap}.flows b{font-size:14px;min-width:280px}.flows code{font-size:13px;color:#dac8f0}.flows em{font-style:normal;font-size:12px;color:#87d4b5;margin-left:auto}.table{overflow:auto}table{border-collapse:collapse;width:100%;font-size:14px;margin:20px 0}th,td{border-bottom:1px solid #33293a;padding:14px 12px;text-align:left;vertical-align:top}th{color:#b9a4ce;font-size:12px}td{color:#c9bfd2}footer{padding:30px 0 60px;font-size:12px;color:#a99ab7}li{margin:8px 0}@media(max-width:760px){.wrap{padding:0 18px}.copy,.pairs{grid-template-columns:1fr}header{padding-top:38px}.flows{padding:12px}.flows b{min-width:0}h2{font-size:24px}nav .wrap{gap:14px}table{min-width:640px}}
</style></head><body><header><div class="wrap"><div class="kicker">NYXID / SCOPE REVIEW / V4.2 · UPDATED 2026.10.21</div><h1>官网裁剪方案。<br><span>与 v4.1 逐项对照。</span></h1><p class="lead">保留已批准的视觉与页面内容，按产品负责人结论收敛本期范围：静态 Blog、Discord、邮件联系，方案逐个验收后开放。</p><p class="en">Retain the approved visual direction; narrow this release to reviewed static content and available entry points.</p><p class="note">2026-10-21 范围修订：本期隐藏、保留实现、验收后配置开放；未修改 v4.1。图片来自 v4.1 模板静态切片与 v4.2 提案；没有动画、iframe、联网脚本或自动打开的站点。点击图片可查看大图。</p><div class="tags"><span>内容与样式基线：v4.1</span><span>本轮：六项范围裁剪</span><span>中英文切片</span><span>保留原尺寸静态工具墙</span></div><p><a href="../2026-10-16-website-scope-cut-v4.2.md">完整方案、验收与依赖 ↗</a>　<a href="evidence.json">生成及截图证据 ↗</a></p></div></header><nav><div class="wrap"><a href="#C01">Blog</a><a href="#C02">订阅</a><a href="#C03">Contact</a><a href="#C04">Footer</a><a href="#C05">Dashboard</a><a href="#C06">Use case</a><a href="#scope">交付边界</a></div></nav><main class="wrap">__SECTIONS__
<section id="scope"><div class="kicker">DELIVERY BOUNDARY</div><h2>这一版何时可以算完成</h2><div class="table"><table><thead><tr><th>阶段</th><th>可验收成果</th><th>不作完成承诺的部分</th></tr></thead><tbody><tr><td>本次评审</td><td>六项范围、前后对照、实施边界、验收条件</td><td>默认关闭画面保持；源码保留，工程证据见本轮交接</td></tr><tr><td>下一步实施</td><td>静态 Blog、默认隐藏无服务承接入口并保留实现、九项默认禁用、真实邮箱和现有认证回归</td><td>CMS、订阅后台、公共 NyxBot、登录默认跳转改造</td></tr><tr><td>逐方案开放</td><td>已登录 / 未登录 / 新注册均保留方案上下文，完成配置、授权及真实任务；记录证据后只开放该项</td><td>注册成功或 Telegram 已连接，均不单独等于方案已交付</td></tr></tbody></table></div><p class="why">保持：官网通用登录与开始使用、方案详情浏览、首页轮换与 hover/focus、既定视觉、原尺寸静态工具墙。Contact 聊天默认隐藏不影响 NyxBot 产品页面或已登录的个人助手。</p><p class="note">v4.1 Contact 原型的模拟“已发送”已在当前实现中被真实 mailto 说明取代；本稿继承这项已确认修正。Footer Status 当前已隐藏。两项均不包装为新增实现。背景水印仍是独立未决项。</p></section></main><footer class="wrap">静态评审图只用于范围和排版讨论，不验证真实认证、邮件投递或方案运行。中英文页面切片均从相同基线生成；截图使用系统字体、固定视口、禁用脚本及网络。<br>Static default-off review targets, not fresh runtime captures; retained source and live integrations require separate acceptance.</footer></body></html>'''
html = html.replace('<section id="scope">', '<section id="visibility"><div class="kicker">2026-10-21 / CONFIGURABLE PUBLICATION</div><h2>隐藏是本期发布状态，不是删除能力</h2><p>四个独立开关默认均为 false：<code>VITE_WEBSITE_BLOG_MENU_ENABLED</code>（仅顶部/移动菜单，不改 Footer、直接访问或 SEO），<code>VITE_WEBSITE_NEWSLETTER_ENABLED</code>、<code>VITE_WEBSITE_CONTACT_CHAT_ENABLED</code>、<code>VITE_WEBSITE_STATUS_ENABLED</code>。集中配置见 <code>frontend/src/features/website-a/release-config.ts</code>。开启必须具备真实 adapter/地址与独立验收记录；缺配置时完整构建失败，后端上线不自动开启。</p><p>操作：确认真实服务验收 → 配置单项 endpoint/HTTPS 地址、验收引用与开关 → 完整重建、定向验收 → 发布同一 SHA 的 HTML、JS、Markdown 与路由产物。关闭回滚只将对应开关设 false（方案移除对应 JSON 项），重建发布；不用从旧 Commit 恢复页面。九个方案详情保持可浏览，开通继续逐个配置、默认 disabled。</p><p class="en">Retain source and adapters; publish each feature only after explicit configuration, acceptance and a complete rebuild. Off means no rendered controls, placeholder or feature requests. Real service availability never changes switches automatically.</p><p class="note">现有 after 图片继续表达默认隐藏后的视觉目标，不是本轮最新运行截图，也不证明源码删除或后端可用。Blog 图仅说明有已审核文章时的布局；本轮不批准任何候选正文。普通可切换 tab 的正式内容继续完整提供 Markdown，不受发布隐藏影响。</p><p><a href="visibility-review.html">查看新版 14 张静态视觉对照 ↗</a> · <a href="../2026-10-16-website-scope-cut-v4.2.md">全部开关、依赖和回滚步骤 ↗</a></p></section>' + '<section id="scope">')
(root / 'review.html').write_text(html.replace('__SECTIONS__', ''.join(sections)))
print('Built review.html: six decisions, five desktop pairs, two mobile pairs; no scripts or frames.')
