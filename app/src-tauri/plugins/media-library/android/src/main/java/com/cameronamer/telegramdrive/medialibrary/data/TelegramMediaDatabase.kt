package com.cameronamer.telegramdrive.medialibrary.data

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase
import androidx.room.TypeConverters
import androidx.room.withTransaction

@Database(
    entities = [
        TelegramMediaEntity::class,
        TelegramMediaPeerEntity::class,
        MediaSyncStateEntity::class,
        MediaPlaybackStateEntity::class,
    ],
    version = 1,
    exportSchema = true,
)
@TypeConverters(MediaTypeConverters::class)
abstract class TelegramMediaDatabase : RoomDatabase() {
    abstract fun mediaDao(): MediaDao

    suspend fun withTransactionClearAccount(accountId: Long) = withTransaction {
        mediaDao().deleteAccountPlayback(accountId)
        mediaDao().deleteAccountSyncState(accountId)
        mediaDao().deleteAccountPeers(accountId)
        mediaDao().deleteAccountMedia(accountId)
    }

    companion object {
        const val DATABASE_NAME = "telegram_media_room.db"
        @Volatile private var instance: TelegramMediaDatabase? = null

        fun get(context: Context): TelegramMediaDatabase = instance ?: synchronized(this) {
            instance ?: Room.databaseBuilder(
                context.applicationContext,
                TelegramMediaDatabase::class.java,
                DATABASE_NAME,
            ).build().also { instance = it }
        }

        internal fun replaceForTest(database: TelegramMediaDatabase?) {
            instance = database
        }
    }
}

