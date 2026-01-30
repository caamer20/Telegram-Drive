import {
    File, FileText, FileImage, FileVideo, FileAudio,
    FileArchive, FileCode, FileSpreadsheet, Presentation,
    FileType
} from 'lucide-react';

const extensionMap: Record<string, { icon: typeof File; color: string }> = {
    // Images
    jpg: { icon: FileImage, color: 'text-pink-400' },
    jpeg: { icon: FileImage, color: 'text-pink-400' },
    png: { icon: FileImage, color: 'text-pink-400' },
    gif: { icon: FileImage, color: 'text-pink-400' },
    webp: { icon: FileImage, color: 'text-pink-400' },
    svg: { icon: FileImage, color: 'text-pink-400' },
    bmp: { icon: FileImage, color: 'text-pink-400' },
    heic: { icon: FileImage, color: 'text-pink-400' },

    // Videos
    mp4: { icon: FileVideo, color: 'text-purple-400' },
    mov: { icon: FileVideo, color: 'text-purple-400' },
    avi: { icon: FileVideo, color: 'text-purple-400' },
    mkv: { icon: FileVideo, color: 'text-purple-400' },
    webm: { icon: FileVideo, color: 'text-purple-400' },

    // Audio
    mp3: { icon: FileAudio, color: 'text-green-400' },
    wav: { icon: FileAudio, color: 'text-green-400' },
    flac: { icon: FileAudio, color: 'text-green-400' },
    aac: { icon: FileAudio, color: 'text-green-400' },
    ogg: { icon: FileAudio, color: 'text-green-400' },

    // Documents
    pdf: { icon: FileType, color: 'text-red-400' },
    doc: { icon: FileText, color: 'text-blue-400' },
    docx: { icon: FileText, color: 'text-blue-400' },
    txt: { icon: FileText, color: 'text-gray-400' },
    rtf: { icon: FileText, color: 'text-gray-400' },
    md: { icon: FileText, color: 'text-gray-400' },

    // Spreadsheets
    xls: { icon: FileSpreadsheet, color: 'text-green-500' },
    xlsx: { icon: FileSpreadsheet, color: 'text-green-500' },
    csv: { icon: FileSpreadsheet, color: 'text-green-500' },

    // Presentations
    ppt: { icon: Presentation, color: 'text-orange-400' },
    pptx: { icon: Presentation, color: 'text-orange-400' },
    key: { icon: Presentation, color: 'text-orange-400' },

    // Archives
    zip: { icon: FileArchive, color: 'text-yellow-400' },
    rar: { icon: FileArchive, color: 'text-yellow-400' },
    '7z': { icon: FileArchive, color: 'text-yellow-400' },
    tar: { icon: FileArchive, color: 'text-yellow-400' },
    gz: { icon: FileArchive, color: 'text-yellow-400' },

    // Code
    js: { icon: FileCode, color: 'text-yellow-300' },
    ts: { icon: FileCode, color: 'text-blue-300' },
    jsx: { icon: FileCode, color: 'text-cyan-300' },
    tsx: { icon: FileCode, color: 'text-cyan-300' },
    py: { icon: FileCode, color: 'text-green-300' },
    rs: { icon: FileCode, color: 'text-orange-300' },
    go: { icon: FileCode, color: 'text-cyan-400' },
    java: { icon: FileCode, color: 'text-red-300' },
    html: { icon: FileCode, color: 'text-orange-400' },
    css: { icon: FileCode, color: 'text-blue-400' },
    json: { icon: FileCode, color: 'text-yellow-200' },
};

export function getFileTypeInfo(filename: string): { icon: typeof File; color: string } {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return extensionMap[ext] || { icon: File, color: 'text-telegram-subtext' };
}

interface FileTypeIconProps {
    filename: string;
    className?: string;
}

export function FileTypeIcon({ filename, className = "w-10 h-10" }: FileTypeIconProps) {
    const { icon: Icon, color } = getFileTypeInfo(filename);
    return <Icon className={`${className} ${color}`} />;
}
