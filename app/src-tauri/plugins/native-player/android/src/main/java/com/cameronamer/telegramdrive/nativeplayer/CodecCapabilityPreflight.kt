package com.cameronamer.telegramdrive.nativeplayer

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.os.Build
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.mediacodec.MediaCodecUtil

enum class CapabilityStatus { SUPPORTED, LIKELY_SUPPORTED, UNSUPPORTED, UNKNOWN }
enum class VideoCodecFamily { AVC, HEVC, DOLBY_VISION, UNKNOWN }
enum class HevcProfile { MAIN_8, MAIN_10, UNSUPPORTED, UNKNOWN }
enum class HdrType { SDR, HDR10, HDR10_PLUS, HLG, DOLBY_VISION, UNKNOWN_HDR, UNKNOWN }
enum class ChromaFormat { YUV_420, YUV_422, YUV_444, UNKNOWN }

data class VideoCapabilityMetadata(
    val sampleMimeType: String? = null,
    val codecs: String? = null,
    val width: Int? = null,
    val height: Int? = null,
    val frameRate: Double? = null,
    val averageBitrate: Long? = null,
    val peakBitrate: Long? = null,
    val bitDepth: Int? = null,
    val hdrType: HdrType = HdrType.UNKNOWN,
    val chromaFormat: ChromaFormat = ChromaFormat.UNKNOWN,
    val containerMimeType: String? = null,
    val rotationDegrees: Int? = null,
    val codecProfile: Int? = null,
    val codecLevel: Int? = null,
) {
    val effectiveBitrate: Long? get() = peakBitrate ?: averageBitrate

    companion object {
        @androidx.annotation.OptIn(UnstableApi::class)
        fun fromFormat(format: Format): VideoCapabilityMetadata {
            val profileLevel = try {
                MediaCodecUtil.getCodecProfileAndLevel(format)
            } catch (_: RuntimeException) {
                null
            }
            val color = format.colorInfo
            val codecText = format.codecs
            val hdrType = when {
                format.sampleMimeType.equals(MimeTypes.VIDEO_DOLBY_VISION, true) ||
                    codecText?.contains(Regex("(?i)(^|,|\\s)(dvhe|dvh1)\\.")) == true -> HdrType.DOLBY_VISION
                codecText?.contains("hdr10+", ignoreCase = true) == true -> HdrType.HDR10_PLUS
                color?.colorTransfer == C.COLOR_TRANSFER_ST2084 -> HdrType.HDR10
                color?.colorTransfer == C.COLOR_TRANSFER_HLG -> HdrType.HLG
                color?.hdrStaticInfo != null -> HdrType.UNKNOWN_HDR
                color != null && !androidx.media3.common.ColorInfo.isTransferHdr(color) -> HdrType.SDR
                else -> HdrType.UNKNOWN
            }
            return VideoCapabilityMetadata(
                sampleMimeType = format.sampleMimeType,
                codecs = codecText,
                width = format.width.knownInt(),
                height = format.height.knownInt(),
                frameRate = format.frameRate.takeIf { it > 0f }?.toDouble(),
                averageBitrate = format.averageBitrate.knownInt()?.toLong(),
                peakBitrate = format.peakBitrate.knownInt()?.toLong(),
                bitDepth = color?.lumaBitdepth?.knownInt(),
                hdrType = hdrType,
                chromaFormat = inferChroma(codecText),
                containerMimeType = format.containerMimeType,
                rotationDegrees = format.rotationDegrees.takeIf { it != Format.NO_VALUE },
                codecProfile = profileLevel?.first,
                codecLevel = profileLevel?.second,
            )
        }

        private fun Int.knownInt(): Int? = takeIf { it != Format.NO_VALUE && it > 0 }

        internal fun inferChroma(codecs: String?): ChromaFormat {
            val normalized = codecs?.lowercase() ?: return ChromaFormat.UNKNOWN
            return when {
                Regex("(^|[^0-9])4[:._-]?4[:._-]?4([^0-9]|$)").containsMatchIn(normalized) ||
                    Regex("(^|[^0-9])yuv444([^0-9]|$)").containsMatchIn(normalized) -> ChromaFormat.YUV_444
                Regex("(^|[^0-9])4[:._-]?2[:._-]?2([^0-9]|$)").containsMatchIn(normalized) ||
                    Regex("(^|[^0-9])yuv422([^0-9]|$)").containsMatchIn(normalized) -> ChromaFormat.YUV_422
                Regex("(^|[^0-9])4[:._-]?2[:._-]?0([^0-9]|$)").containsMatchIn(normalized) ||
                    Regex("(^|[^0-9])yuv420([^0-9]|$)").containsMatchIn(normalized) -> ChromaFormat.YUV_420
                else -> ChromaFormat.UNKNOWN
            }
        }
    }
}

data class DecoderCapabilityReport(
    val decoderName: String,
    val hardwareAccelerated: Boolean,
    val softwareOnly: Boolean,
    val vendor: Boolean,
    val alias: Boolean,
    val profileSupported: Boolean?,
    val levelSupported: Boolean?,
    val sizeSupported: Boolean?,
    val rateSupported: Boolean?,
    val bitrateSupported: Boolean?,
)

data class VideoCapabilityResult(
    val status: CapabilityStatus,
    val codecFamily: VideoCodecFamily,
    val codecTag: String?,
    val mimeType: String?,
    val hevcProfile: HevcProfile,
    val hdrType: HdrType,
    val chromaFormat: ChromaFormat,
    val profileStatus: CapabilityStatus,
    val resolutionStatus: CapabilityStatus,
    val frameRateStatus: CapabilityStatus,
    val bitrateStatus: CapabilityStatus,
    val decoderCount: Int,
    val hardwareDecoderCount: Int,
    val softwareDecoderCount: Int,
    val vendorDecoderCount: Int,
    val aliasDecoderCount: Int,
    val reasonCode: String,
    val diagnostic: String,
    val displayVerificationRequired: Boolean,
) {
    companion object {
        fun unknown(reasonCode: String, diagnostic: String) = VideoCapabilityResult(
            CapabilityStatus.UNKNOWN,
            VideoCodecFamily.UNKNOWN,
            null,
            null,
            HevcProfile.UNKNOWN,
            HdrType.UNKNOWN,
            ChromaFormat.UNKNOWN,
            CapabilityStatus.UNKNOWN,
            CapabilityStatus.UNKNOWN,
            CapabilityStatus.UNKNOWN,
            CapabilityStatus.UNKNOWN,
            0,
            0,
            0,
            0,
            0,
            reasonCode,
            diagnostic,
            false,
        )
    }
}

@androidx.annotation.OptIn(UnstableApi::class)
object CodecCapabilityPreflight {
    fun inspect(metadata: VideoCapabilityMetadata): VideoCapabilityResult {
        val mimeType = resolveMimeType(metadata)
            ?: return VideoCapabilityResult.unknown("CODEC_UNKNOWN", "Media3 did not expose a recognized video MIME type or codec tag.")
        val reports = MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos
            .asSequence()
            .filter { !it.isEncoder && it.supportedTypes.any { type -> type.equals(mimeType, true) } }
            .mapNotNull { report(it, mimeType, metadata) }
            .toList()
        return classify(metadata, mimeType, reports)
    }

    internal fun classify(
        metadata: VideoCapabilityMetadata,
        mimeType: String?,
        reports: List<DecoderCapabilityReport>,
    ): VideoCapabilityResult {
        if (mimeType == null) return VideoCapabilityResult.unknown("CODEC_UNKNOWN", "Video codec metadata is incomplete.")
        val family = codecFamily(mimeType, metadata.codecs)
        val hevcProfile = if (family == VideoCodecFamily.HEVC) classifyHevcProfile(metadata) else HevcProfile.UNKNOWN
        val codecTag = metadata.codecs?.trim()?.takeIf(String::isNotEmpty)
        val hdr = if (family == VideoCodecFamily.DOLBY_VISION) HdrType.DOLBY_VISION else metadata.hdrType
        val displayVerification = hdr !in setOf(HdrType.SDR, HdrType.UNKNOWN)

        if (metadata.chromaFormat == ChromaFormat.YUV_422 || metadata.chromaFormat == ChromaFormat.YUV_444) {
            return result(
                CapabilityStatus.UNSUPPORTED, family, codecTag, mimeType, hevcProfile, hdr, metadata,
                reports, CapabilityStatus.UNSUPPORTED, "UNSUPPORTED_CHROMA",
                "The extracted codec metadata reports ${metadata.chromaFormat}; Android playback support is not advertised.",
                displayVerification,
            )
        }
        if (reports.isEmpty()) {
            return result(
                CapabilityStatus.UNSUPPORTED, family, codecTag, mimeType, hevcProfile, hdr, metadata,
                reports, CapabilityStatus.UNSUPPORTED, "NO_DECODER",
                "No platform decoder advertises support for $mimeType.", displayVerification,
            )
        }

        val profileStatus = aggregate(reports.map { it.profileSupported })
        val levelStatus = aggregate(reports.filter { it.profileSupported != false }.map { it.levelSupported })
        val sizeStatus = aggregate(reports.filter { it.profileSupported != false && it.levelSupported != false }.map { it.sizeSupported })
        val rateStatus = aggregate(reports.filter { it.profileSupported != false && it.levelSupported != false }.map { it.rateSupported })
        val bitrateStatus = aggregate(reports.filter { it.profileSupported != false && it.levelSupported != false }.map { it.bitrateSupported })

        val unsupportedReason = when {
            family == VideoCodecFamily.HEVC && hevcProfile == HevcProfile.UNSUPPORTED ->
                "UNSUPPORTED_HEVC_PROFILE" to "The extracted HEVC profile is not Main or Main 10."
            profileStatus == CapabilityStatus.UNSUPPORTED ->
                "UNSUPPORTED_PROFILE" to if (hevcProfile == HevcProfile.MAIN_10) {
                    "No decoder advertises the extracted HEVC Main 10 profile."
                } else {
                    "No decoder advertises the extracted video profile."
                }
            levelStatus == CapabilityStatus.UNSUPPORTED ->
                "UNSUPPORTED_LEVEL" to "The extracted codec level exceeds every matching decoder report."
            sizeStatus == CapabilityStatus.UNSUPPORTED ->
                "UNSUPPORTED_RESOLUTION" to "The extracted video resolution exceeds every matching decoder report."
            rateStatus == CapabilityStatus.UNSUPPORTED ->
                "UNSUPPORTED_FRAME_RATE" to "The extracted resolution and frame rate exceed every matching decoder report."
            bitrateStatus == CapabilityStatus.UNSUPPORTED ->
                "UNSUPPORTED_BITRATE" to "The extracted bitrate exceeds every matching decoder report."
            else -> null
        }
        if (unsupportedReason != null) {
            return result(
                CapabilityStatus.UNSUPPORTED, family, codecTag, mimeType, hevcProfile, hdr, metadata,
                reports, profileStatus, unsupportedReason.first, unsupportedReason.second,
                displayVerification, sizeStatus, rateStatus, bitrateStatus,
            )
        }

        val allKnownSupported = listOf(profileStatus, levelStatus, sizeStatus, rateStatus, bitrateStatus)
            .all { it == CapabilityStatus.SUPPORTED }
        val status = if (allKnownSupported && !displayVerification) {
            CapabilityStatus.SUPPORTED
        } else {
            CapabilityStatus.LIKELY_SUPPORTED
        }
        val reason = when {
            displayVerification -> "Decoder support is reported, but HDR or Dolby Vision display rendering requires device testing."
            hevcProfile == HevcProfile.UNKNOWN -> "An HEVC decoder exists, but the extracted profile is unknown; Main 10 is not assumed."
            metadata.width == null || metadata.height == null -> "A decoder exists, but the extracted resolution is unknown."
            metadata.frameRate == null -> "A decoder exists, but the extracted frame rate is unknown."
            metadata.effectiveBitrate == null -> "A decoder exists, but the extracted bitrate is unknown."
            else -> "The platform reports a matching decoder profile and media limits."
        }
        return result(
            status, family, codecTag, mimeType, hevcProfile, hdr, metadata, reports,
            profileStatus, if (status == CapabilityStatus.SUPPORTED) "SUPPORTED" else "LIKELY_SUPPORTED",
            reason, displayVerification, sizeStatus, rateStatus, bitrateStatus,
        )
    }

    private fun result(
        status: CapabilityStatus,
        family: VideoCodecFamily,
        codecTag: String?,
        mimeType: String,
        hevcProfile: HevcProfile,
        hdrType: HdrType,
        metadata: VideoCapabilityMetadata,
        reports: List<DecoderCapabilityReport>,
        profileStatus: CapabilityStatus,
        reasonCode: String,
        diagnostic: String,
        displayVerificationRequired: Boolean,
        resolutionStatus: CapabilityStatus = aggregate(reports.map { it.sizeSupported }),
        frameRateStatus: CapabilityStatus = aggregate(reports.map { it.rateSupported }),
        bitrateStatus: CapabilityStatus = aggregate(reports.map { it.bitrateSupported }),
    ) = VideoCapabilityResult(
        status,
        family,
        codecTag,
        mimeType,
        hevcProfile,
        hdrType,
        metadata.chromaFormat,
        profileStatus,
        resolutionStatus,
        frameRateStatus,
        bitrateStatus,
        reports.size,
        reports.count { it.hardwareAccelerated },
        reports.count { it.softwareOnly },
        reports.count { it.vendor },
        reports.count { it.alias },
        reasonCode,
        diagnostic,
        displayVerificationRequired,
    )

    private fun aggregate(values: List<Boolean?>): CapabilityStatus = when {
        values.isEmpty() || values.all { it == null } -> CapabilityStatus.UNKNOWN
        values.any { it == true } -> CapabilityStatus.SUPPORTED
        values.all { it == false } -> CapabilityStatus.UNSUPPORTED
        else -> CapabilityStatus.UNKNOWN
    }

    private fun report(
        info: MediaCodecInfo,
        mimeType: String,
        metadata: VideoCapabilityMetadata,
    ): DecoderCapabilityReport? = try {
        val capabilities = info.getCapabilitiesForType(
            info.supportedTypes.first { it.equals(mimeType, true) },
        )
        val profileLevels = capabilities.profileLevels.toList()
        val profileSupported = metadata.codecProfile?.let { requested ->
            profileLevels.any { it.profile == requested }
        } ?: when (classifyHevcProfile(metadata)) {
            HevcProfile.MAIN_8 -> profileLevels.any { it.profile == MediaCodecInfo.CodecProfileLevel.HEVCProfileMain }
            HevcProfile.MAIN_10 -> profileLevels.any { it.profile in main10Profiles() }
            HevcProfile.UNSUPPORTED -> false
            HevcProfile.UNKNOWN -> null
        }
        val levelSupported = if (metadata.codecProfile != null && metadata.codecLevel != null) {
            profileLevels.any { it.profile == metadata.codecProfile && it.level >= metadata.codecLevel }
        } else null
        val video = capabilities.videoCapabilities
        val sizeSupported = if (video != null && metadata.width != null && metadata.height != null) {
            video.isSizeSupported(metadata.width, metadata.height)
        } else null
        val rateSupported = if (
            video != null && metadata.width != null && metadata.height != null &&
            metadata.frameRate != null && metadata.frameRate > 0
        ) {
            video.areSizeAndRateSupported(metadata.width, metadata.height, metadata.frameRate)
        } else null
        val bitrateSupported = if (video != null) {
            metadata.effectiveBitrate?.let { it <= Int.MAX_VALUE && video.bitrateRange.contains(it.toInt()) }
        } else null
        val softwareName = info.name.startsWith("OMX.google.", true) ||
            info.name.startsWith("c2.android.", true) || info.name.startsWith("c2.google.", true)
        DecoderCapabilityReport(
            decoderName = info.name,
            hardwareAccelerated = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) info.isHardwareAccelerated else !softwareName,
            softwareOnly = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) info.isSoftwareOnly else softwareName,
            vendor = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) info.isVendor else !softwareName,
            alias = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && info.isAlias,
            profileSupported = profileSupported,
            levelSupported = levelSupported,
            sizeSupported = sizeSupported,
            rateSupported = rateSupported,
            bitrateSupported = bitrateSupported,
        )
    } catch (_: RuntimeException) {
        null
    }

    private fun main10Profiles(): Set<Int> = buildSet {
        add(MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10)
        add(MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10HDR10)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            add(MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10HDR10Plus)
        }
    }

    internal fun classifyHevcProfile(metadata: VideoCapabilityMetadata): HevcProfile {
        metadata.codecProfile?.let { profile ->
            if (profile == MediaCodecInfo.CodecProfileLevel.HEVCProfileMain) return HevcProfile.MAIN_8
            if (profile in main10Profiles()) return HevcProfile.MAIN_10
            return HevcProfile.UNSUPPORTED
        }
        val codec = metadata.codecs?.trim()?.lowercase().orEmpty()
        if (Regex("\\bmain\\s*10\\b").containsMatchIn(codec) ||
            codec.split(',').any { value ->
                val parts = value.trim().split('.')
                parts.firstOrNull() in setOf("hvc1", "hev1") && parts.getOrNull(1) == "2"
            }
        ) return HevcProfile.MAIN_10
        if (Regex("\\bmain(?:\\s*8)?\\b").containsMatchIn(codec) ||
            codec.split(',').any { value ->
                val parts = value.trim().split('.')
                parts.firstOrNull() in setOf("hvc1", "hev1") && parts.getOrNull(1) == "1"
            }
        ) return HevcProfile.MAIN_8
        return HevcProfile.UNKNOWN
    }

    internal fun resolveMimeType(metadata: VideoCapabilityMetadata): String? {
        val sampleMime = metadata.sampleMimeType?.substringBefore(';')?.trim()?.lowercase()
        if (sampleMime in setOf(MimeTypes.VIDEO_H264, MimeTypes.VIDEO_H265, MimeTypes.VIDEO_DOLBY_VISION)) {
            return sampleMime
        }
        return metadata.codecs
            ?.split(',')
            ?.asSequence()
            ?.map(String::trim)
            ?.mapNotNull { codec ->
                when {
                    codec.equals("h.264", true) || codec.equals("h264", true) ||
                        codec.startsWith("avc1", true) || codec.startsWith("avc3", true) -> MimeTypes.VIDEO_H264
                    codec.equals("hevc", true) || codec.equals("h.265", true) || codec.equals("h265", true) ||
                        codec.startsWith("hvc1", true) || codec.startsWith("hev1", true) ||
                        codec.contains(Regex("(?i)\\bmain(?:\\s*10)?\\b")) -> MimeTypes.VIDEO_H265
                    codec.startsWith("dvhe", true) || codec.startsWith("dvh1", true) -> MimeTypes.VIDEO_DOLBY_VISION
                    else -> MimeTypes.getVideoMediaMimeType(codec)
                }
            }
            ?.firstOrNull()
    }

    private fun codecFamily(mimeType: String, codecs: String?): VideoCodecFamily = when {
        mimeType.equals(MimeTypes.VIDEO_H264, true) -> VideoCodecFamily.AVC
        mimeType.equals(MimeTypes.VIDEO_H265, true) -> VideoCodecFamily.HEVC
        mimeType.equals(MimeTypes.VIDEO_DOLBY_VISION, true) ||
            codecs?.startsWith("dvhe", true) == true || codecs?.startsWith("dvh1", true) == true -> VideoCodecFamily.DOLBY_VISION
        else -> VideoCodecFamily.UNKNOWN
    }
}
