const fs = require('fs');
const path = require('path');

const LOCALES_DIR = path.join(__dirname, '../../src/i18n/locales');
const en = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, 'en.json'), 'utf8'));

const TRANSLATIONS = {
  es: {
    auth: {
      desktop_required: "Aplicación de escritorio requerida",
      desktop_required_desc: "Está viendo el servidor de desarrollo interno en un navegador. Esta aplicación requiere acceso al backend del sistema (Rust).",
      open_window_prompt: "Abra la ventana de Telegram Drive en la barra de tareas de su sistema operativo para continuar.",
      tagline: "Almacenamiento seguro en la nube",
      too_many_requests: "Demasiadas solicitudes",
      flood_wait_msg: "Telegram ha limitado temporalmente sus acciones.",
      please_wait: "Espere antes de volver a intentarlo.",
      timer_reset_warning: "No reinicie la aplicación. El temporizador se reiniciará si lo hace.",
      api_credentials: "Credenciales de API",
      api_id: "API ID",
      api_hash: "API Hash",
      configure: "Configurar",
      how_to_get_credentials: "¿Cómo obtengo mis credenciales de API?",
      dev_mode: "Modo desarrollador",
      phone_number: "Número de teléfono",
      qr_code: "Código QR",
      continue: "Continuar",
      back_to_config: "Volver a la configuración",
      scan_qr: "Escanee con su aplicación de Telegram",
      qr_instructions: "Ajustes > Dispositivos > Vincular dispositivo de escritorio",
      waiting_for_scan: "Esperando escaneo...",
      refresh_qr: "Actualizar código QR",
      telegram_code: "Código de Telegram",
      change_phone: "Cambiar número de teléfono",
      two_factor_enabled: "Su cuenta tiene habilitada la autenticación en dos pasos. Ingrese su contraseña en la nube para continuar.",
      cloud_password: "Contraseña en la nube",
      password_placeholder: "Ingrese su contraseña",
      back_to_code: "Volver a ingresar código",
      donate: "Donar",
      getting_started: "Primeros pasos",
      close_help: "Cerrar ayuda",
      privacy_note: "Sus credenciales se almacenan localmente en su dispositivo y nunca se envían a servidores de terceros."
    },
    ads: {
      sponsored: "Patrocinado",
      sponsor_message: "Un mensaje de nuestro patrocinador",
      sponsor_support_desc: "Visitar al patrocinador apoya el desarrollo y mantiene Telegram Drive gratuito.",
      continue_to_files: "Continuar a los archivos",
      browser_note: "El contenido patrocinado se abre en su navegador.",
      close_ad: "Cerrar anuncio"
    }
  },
  it: {
    auth: {
      desktop_required: "È richiesta l'app desktop",
      desktop_required_desc: "Stai visualizzando il server di sviluppo interno in un browser. Questa applicazione non può funzionare qui perché richiede l'accesso al backend di sistema (Rust).",
      open_window_prompt: "Apri la finestra di Telegram Drive dalla barra delle applicazioni o dal dock del sistema operativo per continuare.",
      tagline: "Archiviazione sicura self-hosted",
      too_many_requests: "Troppe richieste",
      flood_wait_msg: "Telegram ha limitato temporaneamente le tue azioni.",
      please_wait: "Attendi prima di riprovare.",
      timer_reset_warning: "Non riavviare l'app. Il timer si reimposterà.",
      api_credentials: "Credenziali API",
      api_id: "ID API",
      api_hash: "Hash API",
      configure: "Configura",
      how_to_get_credentials: "Come ottengo le credenziali API?",
      dev_mode: "Modalità sviluppatore",
      phone_number: "Numero di telefono",
      qr_code: "Codice QR",
      continue: "Continua",
      back_to_config: "Torna alla configurazione",
      scan_qr: "Scansiona con l'app Telegram",
      qr_instructions: "Impostazioni > Dispositivi > Collega dispositivo desktop",
      waiting_for_scan: "In attesa della scansione...",
      refresh_qr: "Aggiorna codice QR",
      telegram_code: "Codice Telegram",
      change_phone: "Cambia numero di telefono",
      two_factor_enabled: "Il tuo account ha l'autenticazione a due fattori attiva. Inserisci la password cloud per continuare.",
      cloud_password: "Password cloud",
      password_placeholder: "Inserisci la password",
      back_to_code: "Torna all'inserimento del codice",
      donate: "Dona",
      getting_started: "Per iniziare",
      close_help: "Chiudi guida",
      privacy_note: "Le tue credenziali sono archiviate localmente sul dispositivo e non vengono mai inviate a server di terze parti. Tutti i dati passano direttamente tra te e Telegram."
    },
    ads: {
      sponsored: "Sponsorizzato",
      sponsor_message: "Un breve messaggio dal nostro sponsor",
      sponsor_support_desc: "Visitare lo sponsor aiuta a sostenere lo sviluppo e mantiene Telegram Drive disponibile gratuitamente.",
      continue_to_files: "Continua ai file",
      browser_note: "Il contenuto sponsorizzato si apre nel browser. Vedrai questo passaggio una sola volta.",
      close_ad: "Chiudi annuncio"
    }
  },
  ru: {
    auth: {
      desktop_required: "Требуется приложение для ПК",
      desktop_required_desc: "Вы просматриваете внутренний сервер разработки в браузере. Этому приложению необходим доступ к системному бэкенду (Rust).",
      open_window_prompt: "Откройте окно Telegram Drive в панели задач для продолжения.",
      tagline: "Защищенное облачное хранилище",
      too_many_requests: "Слишком много запросов",
      flood_wait_msg: "Telegram временно ограничил ваши действия.",
      please_wait: "Пожалуйста, подождите перед повторной попыткой.",
      timer_reset_warning: "Не перезапускайте приложение.",
      api_credentials: "Учетные данные API",
      api_id: "API ID",
      api_hash: "API Hash",
      configure: "Настроить",
      how_to_get_credentials: "Как получить учетные данные API?",
      dev_mode: "Режим разработчика",
      phone_number: "Номер телефона",
      qr_code: "QR-код",
      continue: "Продолжить",
      back_to_config: "Назад к настройкам",
      scan_qr: "Отсканируйте с помощью Telegram",
      qr_instructions: "Настройки > Устройства > Привязать устройство",
      waiting_for_scan: "Ожидание сканирования...",
      refresh_qr: "Обновить QR-код",
      telegram_code: "Код Telegram",
      change_phone: "Изменить номер телефона",
      two_factor_enabled: "Для вашего аккаунта включена двухфакторная аутентификация.",
      cloud_password: "Облачный пароль",
      password_placeholder: "Введите пароль",
      back_to_code: "Назад к вводу кода",
      donate: "Пожертвовать",
      getting_started: "Начало работы",
      close_help: "Закрыть справку",
      privacy_note: "Ваши учетные данные хранятся локально на вашем устройстве."
    },
    ads: {
      sponsored: "Спонсор",
      sponsor_message: "Сообщение от спонсора",
      sponsor_support_desc: "Просмотр спонсора помогает поддерживать разработку Telegram Drive.",
      continue_to_files: "Перейти к файлам",
      browser_note: "Спонсорский контент откроется в браузере.",
      close_ad: "Закрыть рекламу"
    }
  }
};

const locales = ['es', 'ru', 'zh-CN', 'fr', 'it', 'ar', 'pt-BR', 'de', 'hi', 'id', 'tr', 'ja', 'ko'];

for (const loc of locales) {
  const filePath = path.join(LOCALES_DIR, `${loc}.json`);
  if (fs.existsSync(filePath)) {
    const json = JSON.parse(fs.readFileSync(filePath, 'utf8'));
    if (TRANSLATIONS[loc]) {
      json.auth = TRANSLATIONS[loc].auth;
      json.ads = TRANSLATIONS[loc].ads;
    } else {
      // Draft translation mapping
      json.auth = JSON.parse(JSON.stringify(en.auth));
      json.ads = JSON.parse(JSON.stringify(en.ads));
    }
    fs.writeFileSync(filePath, JSON.stringify(json, null, 2) + '\n', 'utf8');
    console.log(`Synced auth and ads to ${loc}.json`);
  }
}
