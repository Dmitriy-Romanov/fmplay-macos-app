# FMPLAY macOS

Минимальное macOS-приложение на Rust для прослушивания радио с `fmplay.ru`.

## Запуск

```sh
cargo run
```

Приложение загружает каталог `https://fmplay.ru/stations.json`, показывает станции, ищет по названию и переключает качество `16 / 24 / 32 / 48` кбит/с. По умолчанию выбран `48`.

Избранное сохраняется в:

```text
~/Library/Application Support/FMPLAY Radio/config.json
```

## Сборка

```sh
cargo build --release
```

Готовый бинарник будет в `target/release/fmplay_macos_app`.

## macOS app

Чтобы собрать `.app` с иконкой FMPLAY:

```sh
scripts/build_app.sh
```

Готовое приложение будет в:

```text
target/release/FMPLAY Radio.app
```
