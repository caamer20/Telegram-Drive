package com.cameronamer.telegramdrive.nativeplayer

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.os.Build

enum class CapabilityStatus { SUPPORTED, UNSUPPORTED, UNKNOWN }

data class VideoCapabilityMetadata(
    val codec: String?,
    val width: Int?,
    val height: Int?,
    val frameRate: Double?,
    val bitrate: Long?,
    val bitDepth: Int?,
    val hdr: Boolean?,
)

data class DecoderCapabilityReport(
    val decoderName: String,
    val main8: Boolean,
    val main10: Boolean,
    val sizeAndRateSupported: Boolean?,
    val bitrateSupported: Boolean?,
)

data class VideoCapabilityResult(
    val status: CapabilityStatus,
    val codecTag: String?,
    val mimeType: String?,
    val reason: String,
)

object CodecCapabilityPreflight {
    fun inspect(metadata: VideoCapabilityMetadata): VideoCapabilityResult {
        val codecTag = metadata.codec?.trim()?.lowercase()
        val mimeType = mimeForCodec(codecTag)
            ?: return VideoCapabilityResult(CapabilityStatus.UNKNOWN, codecTag, null, "codec metadata is incomplete")

        val reports = MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos
            .asSequence()
            .filter { !it.isEncoder && it.supportedTypes.any { type -> type.equals(mimeType, true) } }
            .mapNotNull { info -> report(info, mimeType, metadata) }
            .toList()
        return classify(metadata, codecTag, mimeType, reports)
    }

    internal fun classify(
        metadata: VideoCapabilityMetadata,
        codecTag: String?,
        mimeType: String?,
        reports: List<DecoderCapabilityReport>,
    ): VideoCapabilityResult {
        if (mimeType == null) {
            return VideoCapabilityResult(CapabilityStatus.UNKNOWN, codecTag, null, "codec metadata is incomplete")
        }
        if (reports.isEmpty()) {
            return VideoCapabilityResult(CapabilityStatus.UNSUPPORTED, codecTag, mimeType, "no platform decoder reports this codec")
        }
        val needsMain10 = metadata.bitDepth == 10 || metadata.hdr == true
        val profileCandidates = reports.filter { if (needsMain10) it.main10 else it.main8 || it.main10 }
        if (profileCandidates.isEmpty()) {
            return VideoCapabilityResult(
                CapabilityStatus.UNSUPPORTED,
                codecTag,
                mimeType,
                if (needsMain10) "no decoder reports Main 10 support" else "no compatible decoder profile was reported",
            )
        }
        if (profileCandidates.any { it.sizeAndRateSupported == true && it.bitrateSupported != false }) {
            return VideoCapabilityResult(CapabilityStatus.SUPPORTED, codecTag, mimeType, "device reports compatible profile, size, rate, and bitrate")
        }
        if (profileCandidates.all { it.sizeAndRateSupported == false || it.bitrateSupported == false }) {
            return VideoCapabilityResult(CapabilityStatus.UNSUPPORTED, codecTag, mimeType, "resolution, frame rate, or bitrate exceeds reported decoder limits")
        }
        return VideoCapabilityResult(CapabilityStatus.UNKNOWN, codecTag, mimeType, "decoder exists but media metadata or device limits are incomplete")
    }

    private fun report(
        info: MediaCodecInfo,
        mimeType: String,
        metadata: VideoCapabilityMetadata,
    ): DecoderCapabilityReport? = try {
        val capabilities = info.getCapabilitiesForType(mimeType)
        val profiles = capabilities.profileLevels.map { it.profile }.toSet()
        val main10Profiles = mutableSetOf(
            MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10,
            MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10HDR10,
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            main10Profiles += MediaCodecInfo.CodecProfileLevel.HEVCProfileMain10HDR10Plus
        }
        val main10 = profiles.any(main10Profiles::contains)
        val main8 = if (mimeType == "video/hevc") {
            profiles.contains(MediaCodecInfo.CodecProfileLevel.HEVCProfileMain)
        } else {
            profiles.isNotEmpty()
        }
        val video = capabilities.videoCapabilities
        val sizeAndRate = if (video != null && metadata.width != null && metadata.height != null) {
            if (metadata.frameRate != null && metadata.frameRate > 0) {
                video.areSizeAndRateSupported(metadata.width, metadata.height, metadata.frameRate)
            } else {
                video.isSizeSupported(metadata.width, metadata.height)
            }
        } else null
        val bitrate = if (video != null) {
            metadata.bitrate?.let { it <= Int.MAX_VALUE && video.bitrateRange.contains(it.toInt()) }
        } else null
        DecoderCapabilityReport(info.name, main8, main10, sizeAndRate, bitrate)
    } catch (_: Exception) {
        null
    }

    private fun mimeForCodec(codec: String?): String? = when (codec) {
        "avc", "avc1", "avc3", "h264", "video/avc" -> "video/avc"
        "hevc", "h265", "hvc1", "hev1", "video/hevc" -> "video/hevc"
        else -> null
    }
}
