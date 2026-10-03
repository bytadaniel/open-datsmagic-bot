# player_2 (Rust)

Отдельный автономный клиент; он не входит в бинарник сервера. По умолчанию подключается к `http://127.0.0.1:8080` и читает `X-Auth-Token` из `token.txt`.

```sh
cd lib/arena-bots/rust_bytadaniel
cargo run --release
```

Параметры окружения:

- `DATS_SERVER_URL` — origin HTTP-сервера, например `http://127.0.0.1:8080`.
- `DATS_PLAYER_TOKEN` — токен напрямую из окружения (приоритетнее файла).
- `DATS_PLAYER_TOKEN_FILE` — путь к токену (по умолчанию `token.txt`, используется если `DATS_PLAYER_TOKEN` не задан).
- `DATS_PLAN_HORIZON_SECONDS` — горизонт сканирования в секундах (по умолчанию 30 для `stable-profit`; для `agile-top1` по умолчанию 15 и ограничен сверху 15).
- `DATS_PLAYER_STRATEGY` — `stable-profit` (по умолчанию, основная стратегия) или `agile-top1` (каждый тик выбирает новый top-1 без удержания, горизонт по умолчанию 15 с и максимум 15 с).
- `DATS_MOVEMENT_STRATEGY` — `none` (по умолчанию) или `survival`: независимо от aim проверяет риск выхода за карту/попадания в притягивающую аномалию и при высокой угрозе ищет безопасное направление.
- `DATS_DOOM_POLICY` — `collect` (по умолчанию: собрать максимум доступных монет до гибели) или `fastest` (сохранить поведение максимально быстрой смерти), используется movement-стратегией `survival`, если выход не найден в 30-секундном скане.
- `DATS_PLAYER_TELEMETRY_FILE` — необязательный общий путь к JSON-логам игрока для визуализатора; по умолчанию token-scoped файл создаётся в системном temp-каталоге.

Быстро переключить режим можно при запуске:

```sh
DATS_PLAYER_STRATEGY=stable-profit cargo run --release
DATS_PLAYER_STRATEGY=agile-top1 cargo run --release
DATS_PLAYER_STRATEGY=agile-top1 DATS_MOVEMENT_STRATEGY=survival DATS_DOOM_POLICY=collect cargo run --release
DATS_PLAYER_TOKEN='none_top1' DATS_PLAYER_STRATEGY=agile-top1 DATS_MOVEMENT_STRATEGY=none cargo run --release
```

Минимальный встроенный HTTP-клиент принимает только `http://` origin без пути; это подходит для локального Rust-сервера и не тянет TLS/HTTP-крейты в отдельный бинарник.

Клиент использует только `POST /play/magcarp/player/move`. Он измеряет RTT, плавно учитывает задержку перед применением команды и ограничивает частоту запросов одним запросом с командами на игровой тик. Каждый живой транспорт получает вектор длины `maxAccel`.
