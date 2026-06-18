import { useEffect, useState } from "react";

/**
 * 圆形用户头像。
 *
 * 行为：
 *   1. `avatarUrl` 为有效 http(s) URL 时，渲染 `<img>`，使用 `object-cover`
 *      以"短边铺满 + 长边居中裁剪"的方式占满圆形容器。这样既能尽量完整地
 *      呈现头像（不会拉伸变形），又不会留出空白边——OAuth `/openapi/me`
 *      返回的头像比例不固定，单纯 `object-contain` 会出现 letterbox。
 *   2. URL 为 `null`/空串/非 http(s) 协议，或 `<img>` onError 触发后，回退
 *      为首字母 + 紫色背景的 placeholder（与登录页 / 控制台 / 设置弹窗的
 *      历史 fallback 视觉一致）。
 *   3. 无尺寸/字号默认值——这是个 layout primitive，由调用方通过
 *      `className` 决定圆的大小（`h-10 w-10` 等），通过 `fallbackTextClassName`
 *      决定首字母字号，避免组件内部硬编码与 ConsolePage / Dialog 的不同
 *      尺寸打架。
 */
export interface UserAvatarProps {
  /** OAuth `/openapi/me` 的 `avatar_url`。允许 null/undefined/空串。 */
  avatarUrl?: string | null;
  /** 用于 placeholder 的源文本（一般是 name；为空就退到 email）。 */
  name?: string | null;
  /** 同上的退路；都没有就显示 "U"。 */
  email?: string | null;
  /** 圆形容器尺寸 + 阴影等。例：`"h-10 w-10"`。 */
  className?: string;
  /** placeholder 文字字号。例：`"text-base"`、`"text-lg"`。 */
  fallbackTextClassName?: string;
}

/**
 * `avatar_url` 是否值得交给 `<img>` 去加载。
 *
 * 拒绝：null / undefined / 空白串 / 非 http(s) 协议（防止 javascript:、data:
 * 之类的奇怪 payload；OFox 后端目前只发 https 头像，但前端不应假设）。
 */
function isUsableAvatarUrl(s: string | null | undefined): s is string {
  if (typeof s !== "string") return false;
  const trimmed = s.trim();
  if (trimmed.length === 0) return false;
  return /^https?:\/\//i.test(trimmed);
}

/** 把 name/email 折成单个大写首字母作为 placeholder。中文取首字本身。 */
function deriveInitial(name?: string | null, email?: string | null): string {
  const src = (name ?? "").trim() || (email ?? "").trim();
  if (!src) return "U";
  // 中文/CJK 直接用首字（charAt 在 BMP 范围内可用；OFox 用户名极少跨越
  // surrogate pair，这里走简单路径足够）。
  const first = src.charAt(0);
  return /[a-z]/i.test(first) ? first.toUpperCase() : first;
}

export function UserAvatar({
  avatarUrl,
  name,
  email,
  className = "h-10 w-10",
  fallbackTextClassName = "text-base",
}: UserAvatarProps) {
  // `<img>` 加载失败时翻成 true，下次 render 走 placeholder 分支。url 变化
  // 时清回 false——用户重新登录或刷新带上了新的 avatar_url 应该再试一次。
  const [imgFailed, setImgFailed] = useState(false);
  useEffect(() => {
    setImgFailed(false);
  }, [avatarUrl]);

  const showImage = isUsableAvatarUrl(avatarUrl) && !imgFailed;

  if (showImage) {
    return (
      <div
        className={`shrink-0 overflow-hidden rounded-full bg-muted ${className}`}
      >
        <img
          src={avatarUrl as string}
          alt={name ?? email ?? "用户头像"}
          // object-cover：保持比例 + 短边铺满 + 长边居中裁剪，圆形容器不留
          // 空白；referrerpolicy=no-referrer 避免某些 CDN 因 referer 校验
          // 拒绝跨站请求（OFox CDN 不强制要求，但成本几乎为零）。
          referrerPolicy="no-referrer"
          loading="lazy"
          draggable={false}
          onError={() => setImgFailed(true)}
          className="h-full w-full object-cover object-center"
        />
      </div>
    );
  }

  // Placeholder：和原本散落在 ConsolePage/OfoxSettingsDialog 各处的紫色
  // 首字母圆保持视觉一致，统一收口在这里。
  return (
    <div
      className={`flex shrink-0 items-center justify-center rounded-full bg-purple-200 font-bold text-purple-700 ${fallbackTextClassName} ${className}`}
    >
      {deriveInitial(name, email)}
    </div>
  );
}
