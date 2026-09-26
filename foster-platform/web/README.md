# 寄养系统前端

前端开发与预览固定监听 `http://127.0.0.1:5173`；后端 API 监听 `http://127.0.0.1:8080`。端口 5173 被占用时，Vite 会直接报错，不会自动换端口。

在本目录运行 `npm install` 后，可用 `npm run dev` 启动开发前端；`npm run build` 后用 `npm run preview` 运行生产构建预览。后端需要单独启动。

Vite 的开发与预览代理会原样转发 `/admin/*`、`/public/login/*`、`/r/*`、`/healthz`、`/readyz` 和 `/agent/ws` 到 `http://127.0.0.1:8080`。其中 `/agent/ws` 支持 WebSocket 升级；SSE 响应保持流式传输，不做内容转换。`/admin`、`/login/:token`、`/service/:token` 由前端页面处理。

外部 NATAPP 隧道只指向前端监听地址 `127.0.0.1:5173`，由前端同源代理访问后端。该代理是 Vite 开发/预览服务的能力；正式部署若换用其他 Web 服务器，需配置相同的路径转发规则。
