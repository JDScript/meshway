import { uiTranslations } from "fumadocs-ui/i18n"

import { i18n } from "./i18n"

/** Fumadocs' own words in the site's languages (English is its default). */
export const translations = i18n
  .translations()
  .extend(uiTranslations())
  .add({
    zh: {
      displayName: "中文",
      "Search(search trigger)": "搜索",
      "Search(search dialog)": "搜索文档",
      "No results found(search dialog)": "没有结果",
      "Open Search(search trigger)(aria-label)": "打开搜索",
      "Close Search(search dialog)(aria-label)": "关闭搜索",
      "On this page(table of contents)": "本页内容",
      "No Headings(table of contents)": "没有标题",
      "Table of Contents(inline table of contents)": "目录",
      "Next Page(pagination)": "下一页",
      "Previous Page(pagination)": "上一页",
      "Choose a language(language switcher)": "选择语言",
      "Choose a language(language switcher)(aria-label)": "选择语言",
      "Language(language switcher)": "语言",
      "Theme(site menu)": "主题",
      "Toggle Theme(theme switcher)(aria-label)": "切换主题",
      "Light(theme switcher)(aria-label)": "浅色",
      "Dark(theme switcher)(aria-label)": "深色",
      "System(theme switcher)(aria-label)": "跟随系统",
      "Toggle Menu(home layout header)(aria-label)": "打开菜单",
      "Open Sidebar(aria-label)": "打开侧栏",
      "Close Sidebar(aria-label)": "关闭侧栏",
      "Open Sidebar(sidebar)(aria-label)": "打开侧栏",
      "Close Sidebar(sidebar)(aria-label)": "关闭侧栏",
      "Collapse Sidebar(sidebar)(aria-label)": "收起侧栏",
      "Show Sidebar(sidebar)": "显示侧栏",
      "Hide Sidebar(sidebar)": "隐藏侧栏",
      "Copy Text(code block)(aria-label)": "复制",
      "Copied Text(code block)(aria-label)": "已复制",
      "Copy Anchor Link(heading anchor)(aria-label)": "复制链接",
      "Copied Anchor Link(heading anchor)(aria-label)": "已复制链接",
      "Copy Link(accordion)(aria-label)": "复制链接",
      "Copied Link(accordion)(aria-label)": "已复制链接",
      "Page Not Found(404 not found page)": "页面不存在",
      "Back to Home(404 not found page)": "返回首页",
      "The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.(404 not found page)":
        "这个页面可能已删除、改名，或暂时不可用。",
      "Last updated on(page footer)": "最后更新于",
      "Type(type table)": "类型",
      "Default(type table)": "默认值",
      "Parameters(type table)": "参数",
      "Returns(type table)": "返回",
      "Prop(type table)": "属性",
    },
    en: { displayName: "English" },
  })
