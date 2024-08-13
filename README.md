# Geektime Rust 语言训练营

第8周：异次元之门：灵活嵌入各种语法 - dino

第15周作业6：
根据要求补充了js worker -> pool, 每个 js worker 在单独 thread 里运行。axum 通过 mpsc channel 发送 Req, js worker 把 Res 通过 oneshot channel 发回代码。
