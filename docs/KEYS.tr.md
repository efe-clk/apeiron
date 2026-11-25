> **English:** [KEYS.md](KEYS.md)

# Anahtar Yönetimi

> **Durum: Tamamlandı.** Şifreli bir depo (FORMAT.tr.md §14) bir parolayla, bir kurtarma anahtarıyla ya da artık bir anahtar yönetim hizmetinin tuttuğu bir anahtarla açılır: HashiCorp Vault, AWS KMS, Google Cloud KMS ya da PKCS#11 üzerinden bir donanım güvenlik modülü. Bir kurtarma anahtarı, kullanmak için birkaç kişinin bir araya gelmesi gerekecek şekilde bölünebilir. Arkasındaki karar ADR-060'tır ([DECISIONS.tr.md](DECISIONS.tr.md)).

## 1. Komutlar

| Komut | Ne yapar |
| --- | --- |
| `apeiron key add --external <provider>:<key>` | Bir hizmetin tuttuğu bir anahtar ekler (§2); eklenmeden önce bir kez denenir |
| `apeiron key rotate <id> [--to <provider>:<key>]` | Bir dış anahtarı, yeni bir anahtar-şifreleme anahtarıyla, aynı hizmetin ya da `--to` ile başka birinin tuttuğu yenisiyle değiştirir, sonra eskisini siler |
| `apeiron key split [--shares <n>] [--threshold <k>]` | Yeni bir kurtarma anahtarı ekler ve onu yalnızca herhangi `k`'si (3) onu geri veren `n` parça (varsayılan 5) olarak gösterir (§3) |
| `apeiron key join <share>...` | Bir kurtarma anahtarını parçalarından birleştirir ve gösterir |
| `apeiron key list` | Her anahtarı türüyle; bir dış anahtarı hizmeti ve anahtarıyla gösterir |

Şifreli bir depoyu `--password-file` olmadan açan bir komut önce dış anahtarlarının hizmetlerine sırayla sorar ve parolayı yalnızca hiçbiri açamazsa sorar. Arka plan hizmetinin `password_file`'ı olmayan bir işi de aynısını yapar; böylece bir sunucu diskinde parola olmadan deposunu açabilir.

## 2. Bir hizmetin tuttuğu anahtarlar

Deponun kendi anahtarları (`id_key` ve `data_key`) bilgisayardan asla çıkmaz. Bir dış anahtar dosyası onları 32 rastgele baytlık bir anahtar-şifreleme anahtarıyla mühürler ve hizmet yalnızca bu anahtar-şifreleme anahtarını, kendi anahtarıyla sarılmış olarak tutar (FORMAT.tr.md §14.2). Açmak, depoyu açanın kimlik bilgileriyle hizmetten onu açmasını ister; bu yüzden hizmette bir anahtarı ya da iznini kaldırmak, depoyu o kişiye kapatır.

`apeiron key rotate` yeni bir anahtar-şifreleme anahtarı ve yeni bir anahtar dosyası yapar ve eskisini siler; `data_key` değişmediği için veri yeniden şifrelenmez. Hizmetin kendi anahtarını yenilemek, Vault'un `rotate`'i ve AWS'nin yıllık yenilemesinin yaptığı gibi, Apeiron'dan bir şey istemez: eski sürümleri açmaya devam ederler.

### 2.1 HashiCorp Vault

`vault:<name>` ya da `vault:<mount>/<name>`: varsayılan olarak `transit/` altındaki transit gizli motorunun bir anahtarı. Adres ve belirteç, Vault'un kendi komutunda olduğu gibi `VAULT_ADDR` ve `VAULT_TOKEN`'dan, ayarlıysa `VAULT_NAMESPACE`'ten gelir. Geliştirme kipindeki Vault 1.20 ile denendi; CI her push'ta dener.

### 2.2 AWS KMS

`aws-kms:<anahtar kimliği, ARN ya da takma ad>`. Kimlik bilgileri `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` ve `AWS_SESSION_TOKEN`'dan, bölge ARN'den ya da `AWS_REGION`'dan gelir; `AWS_ENDPOINT_URL_KMS` ya da `AWS_ENDPOINT_URL` başka bir uç noktayı adlandırır. Profiller, tek oturum açma ve örnek rolleri okunmaz: kimlik bilgileri değişkenlerle verilir, örneğin `aws configure export-credentials --format env` ile. LocalStack 4.9 ile denendi; CI her push'ta dener.

### 2.3 Google Cloud KMS

`gcp-kms:projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>`. Erişim belirteci `GOOGLE_OAUTH_ACCESS_TOKEN`'dan, yoksa `gcloud auth print-access-token`'dan gelir. Henüz denenmedi: Cloud KMS'in bir öykünücüsü yok.

### 2.4 PKCS#11

`pkcs11:module=<kütüphane>,token=<etiket>,key=<etiket>`: bir donanım güvenlik modülünde, belirtecin ve anahtarın etiketleriyle bulunan bir AES anahtarı; kullanıcı PIN'i `APEIRON_PKCS11_PIN`'den gelir. Anahtar-şifreleme anahtarı modülün içinde AES-GCM ile şifrelenir; böylece AES anahtarı modülden asla çıkmaz. SoftHSM 2.7 ile denendi.

## 3. Bir kurtarma anahtarını bölmek

Bir kurtarma anahtarı depoyu tek başına açar; bu yüzden onu saklayan herkes açabilir. `apeiron key split` yeni bir kurtarma anahtarı ekler ve onu yalnızca Shamir'in gizli paylaşımının parçaları olarak gösterir: `n` parçanın herhangi `k`'si anahtarı `apeiron key join` ile geri verir, daha azı onun hakkında hiçbir şey söylemez. Parçalar saklanmaz; her biri, farklı bir kişiye verilmek üzere bir kez gösterilir.

Bir parça `<k>-<x>-<hex>-<check>` biçimindedir; sağlama, geri kalanın BLAKE3 özetinin bir parçasıdır; böylece yanlış yazılmış bir parça yanlış bir anahtar vermek yerine reddedilir. Parçalar kurtarma anahtarının 32 baytından, GF(256)'da her bayt için rastgele bir çok terimliyle yapılır.

## 4. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Veriyi yeni bir `data_key` ile yeniden şifrelemek | Planlanmadı: her pack'i yeniden yazardı; bunun yerine depoyu yenisine kopyalayın |
| AWS profilleri, tek oturum açma ve örnek rolleri | Daha sonra; o zamana kadar kimlik bilgileri ortamdan geçer |
| Google Cloud hizmet hesabı dosyaları | Daha sonra; o zamana kadar `gcloud` bir belirteç verir |
| Google Cloud KMS'i denemek | İlk kullanıcısıyla |

## 5. Crate'ler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron-core` | Dış anahtar dosyaları: yazmak, listelemek ve açılmış bir anahtarla açmak | Tamamlandı |
| `apeiron-kms` | Hizmetler ve Shamir'in gizli paylaşımı | Tamamlandı |
| `apeiron-cli`, `apeiron-daemon` | Komutlar ve dış anahtarlarla açmak | Tamamlandı |
