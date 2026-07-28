const fs = require('fs');
const path = require('path');

const LOCALES_DIR = path.join(__dirname, '../../src/i18n/locales');

const MISSING_DATA = {
  es: { delete: "Eliminar", revoke: "Revocar", update_available: "Actualización disponible", uploading: "Subiendo..." },
  ru: { delete: "Удалить", revoke: "Отозвать", update_available: "Доступно обновление", uploading: "Загрузка..." },
  'zh-CN': { delete: "删除", revoke: "撤销", update_available: "有可用更新", uploading: "正在上传..." },
  fr: { delete: "Supprimer", revoke: "Révoker", update_available: "Mise à jour disponible", uploading: "Envoi en cours..." },
  it: { delete: "Elimina", revoke: "Revoca", update_available: "Aggiornamento disponibile", uploading: "Caricamento..." },
  ar: { delete: "حذف", revoke: "إلغاء", update_available: "تحديث متاح", uploading: "جاري الرفع..." },
  'pt-BR': { delete: "Excluir", revoke: "Revogar", update_available: "Atualização disponível", uploading: "Enviando..." },
  de: { delete: "Löschen", revoke: "Widerrufen", update_available: "Update verfügbar", uploading: "Wird hochgeladen..." },
  hi: { delete: "हटाएं", revoke: "रद्द करें", update_available: "अपडेट उपलब्ध है", uploading: "अपलोड हो रहा है..." },
  id: { delete: "Hapus", revoke: "Cabut", update_available: "Pembaruan tersedia", uploading: "Mengunggah..." },
  tr: { delete: "Sil", revoke: "İptal Et", update_available: "Güncelleme mevcut", uploading: "Yükleniyor..." },
  ja: { delete: "削除", revoke: "失効", update_available: "アップデートが利用可能です", uploading: "アップロード中..." },
  ko: { delete: "삭제", revoke: "취소", update_available: "업데이트 가능", uploading: "업로드 중..." }
};

for (const [locale, data] of Object.entries(MISSING_DATA)) {
  const filePath = path.join(LOCALES_DIR, `${locale}.json`);
  if (fs.existsSync(filePath)) {
    const json = JSON.parse(fs.readFileSync(filePath, 'utf8'));
    if (!json.common) json.common = {};
    if (!json.settings) json.settings = {};

    json.common.delete = data.delete;
    json.settings.revoke = data.revoke;
    json.settings.update_available = data.update_available;
    json.settings.uploading = data.uploading;

    fs.writeFileSync(filePath, JSON.stringify(json, null, 2) + '\n', 'utf8');
    console.log(`Updated ${locale}.json with missing baseline keys`);
  }
}
