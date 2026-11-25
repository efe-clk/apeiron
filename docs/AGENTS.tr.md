> **English:** [AGENTS.md](AGENTS.md)

# Yapay Zekâ Ajanları

> **Durum: Tamamlandı.** Claude Code ya da Cursor gibi dosyaları düzenleyen bir yapay zekâ ajanı, her adımdan sonra klasörünün bir kontrol noktasını alır ve işi tur ya da oturum olarak geri alınabilir. `apeiron mcp`, ona bir deponun geçmişini Model Context Protocol üzerinden verir. Arkasındaki karar ADR-058'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

## 1. Komutlar

| Komut | Ne yapar |
| --- | --- |
| `apeiron checkpoint save [<folder>] [--session <id>] [--message <text>] [--hook]` | Klasörü (varsayılan: şimdiki) son kontrol noktasından beri değiştiyse bir kontrol noktası olarak kaydeder |
| `apeiron checkpoint list [<folder>]` | Klasörün kontrol noktalarını, en yenisi önce, oturumlarıyla listeler |
| `apeiron checkpoint undo [<folder>] [--session] [--to <checkpoint>] [--dry-run]` | Klasörün, ajanın son turundan önce tuttuğunu tutmasını sağlar; `--session` bütün son oturumundan öncesine; `--to` verilen bir kontrol noktasına |
| `apeiron mcp` | Depoyu standart giriş ve çıkış üzerinden Model Context Protocol ile bir ajana sunar (§4) |

`--repo` (ya da `APEIRON_REPO`) olmadan kontrol noktaları, kullanıcının kendine ait ve ilk gerektiğinde veri klasöründe oluşturulan bir depoya gider (Linux'ta `~/.local/share/apeiron/checkpoints`). Depo verilmezse `apeiron mcp` de aynısını kullanır.

## 2. Kontrol noktaları

Bir kontrol noktası, klasörün `checkpoint/<klasör adı>` ref'ine, ajanların ihtiyacı olan biçimde alınmış bir yedeğidir:

| Kural | Neden |
| --- | --- |
| Son kontrol noktasından beri değişmeyen bir klasör kontrol noktası oluşturmaz | Kancalar her adımdan sonra çalışır ve çoğu hiçbir şeyi değiştirmez |
| `.git`, `@dev`'in hazır kümeleri ve `.gitignore`'un adlandırdıkları dışarıda kalır (CLI.tr.md §4.1) | Git kendi geçmişini tutar, derleme çıktısı yeniden yapılabilir |
| Fidye yazılımı alarmı kapalıdır (CLI.tr.md §4.3) | Birçok dosyayı yeniden yazan bir ajan, kontrol noktalarının varlık nedenidir |
| `checkpoint.session` ve `checkpoint.event` snapshot'ın `meta`'sında tutulur | Geri alma, turları ve oturumları bunlarla bulur |

`--hook`, bir ajanın kancasının standart girişe JSON olarak gönderdiğini okur: `session_id`, `cwd` (klasör), `hook_event_name` ve `tool_name`. Standart çıkışa hiçbir şey yazmaz ve her zaman 0 ile çıkar; böylece bir kontrol noktası ajanı asla durdurmaz; bir sorun standart hataya yazılır.

Claude Code için, bir projenin ya da kullanıcının `.claude/settings.json` dosyasında:

```json
{
  "hooks": {
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "apeiron checkpoint save --hook"}]}],
    "PostToolUse": [{"matcher": "Write|Edit|MultiEdit|Bash",
                     "hooks": [{"type": "command", "command": "apeiron checkpoint save --hook"}]}]
  }
}
```

Kullanıcı bir şey sorduğunda (`UserPromptSubmit`) alınan kontrol noktası bir tur başlatır. Diğer ajanlar adımlarından önce ve sonra `apeiron checkpoint save --session <id>` çalıştırabilir.

## 3. Geri alma

`apeiron checkpoint undo`, klasörün kontrol noktaları arasındaki en yeni oturumu alır ve onun son turunu başlatan kontrol noktasına döner; `--session` oturumun ilk kontrol noktasına, `--to` ise `list`'in gösterdiği herhangi bir kontrol noktasına döner.

1. Klasörün şimdi tuttuğunu `undo` oturumunun bir kontrol noktası olarak kaydet; böylece geri almanın kendisi `--to` ile geri alınabilir.
2. Klasörün, `apeiron switch`'in yaptığı gibi kontrol noktasını tutmasını sağla (BRANCH.tr.md §6): yalnızca farklı olan yazılır ya da silinir ve kontrol noktalarının dışarıda bıraktığı (`.git`, derleme çıktısı) olduğu gibi bırakılır.
3. Sonucu, sonraki kontrol noktalarının devam edeceği `checkpoint: undone` kontrol noktası olarak kaydet.

## 4. Model Context Protocol

`apeiron mcp`, standart giriş ve çıkış üzerinde bir [Model Context Protocol](https://modelcontextprotocol.io) sunucusudur; böylece bir ajan bir deponun geçmişine bakabilir: bir dosyanın önceden ne tuttuğu, iki yedek arasında neyin değiştiği, bir metnin nerede olduğu. Hiçbir şeyi değiştirmez.

| Araç | Verdiği |
| --- | --- |
| `refs` | Ref'ler ve her birinin gösterdiği snapshot |
| `log` | Bir ref'in snapshot'ları, en yenisi önce |
| `list` | Bir snapshot'ın bir yoldaki dosyaları ve klasörleri |
| `read_file` | Bir snapshot'ın bir dosyası metin olarak, en çok 256 KiB |
| `diff`, `diff_text` | İki snapshot arasında değişen yollar ve metin dosyalarının değişen satırları |
| `search` | Bir snapshot'ta ya da bütün geçmişinde ada ve metne göre dosyalar (SEARCH.tr.md) |

Claude Code için: `claude mcp add apeiron -- apeiron mcp` ya da bir depoyla: `claude mcp add apeiron -- apeiron --repo /path/to/repo mcp`. Araçlar `apeiron::json` üzerinden gider; C ABI'nin ve sayfaların işlemleriyle aynıdır.

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| MCP üzerinden depoyu değiştiren araçlar | Daha sonra; bir ajan dosyaları değiştirir, kontrol noktaları onları kaydeder |
| Claude Code dışındaki ajanlar için kancalar | Komut çalıştırabilen her ajan `checkpoint save`'i kullanır; diğerleri için hazır ayarlar kullanıcılarıyla gelir |

## 6. Crate'ler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron` | Bir yedek isteğinin `skip_same`'i, bir switch isteğinin `exclude`'u | Tamamlandı |
| `apeiron-cli` | `apeiron checkpoint`, `apeiron mcp` | Tamamlandı |
