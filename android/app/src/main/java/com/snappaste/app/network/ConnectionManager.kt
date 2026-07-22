package com.snappaste.app.network

import android.util.Log

/**
 * States of the Android-side connection state machine.
 * Mirrors the Desktop-side state machine.
 */
enum class ConnectionState {
    DISCONNECTED,
    TCP_CONNECTED,
    AUTHENTICATED,
    IDLE,
    SENDING_IMAGE
}

/**
 * Events that trigger state transitions on the Android client.
 */
enum class ConnectionEvent {
    TCP_CONNECTED,
    AUTH_SUCCESS,
    ENTER_IDLE,
    START_SEND,
    SEND_COMPLETE,
    
    // Recovery edges
    DISCONNECT_OR_ERROR,
    AUTH_FAILED
}

/**
 * Manages the connection state machine for the Android client.
 */
class ConnectionManager {

    companion object {
        private const val TAG = "ConnectionManager"
    }

    @Volatile
    private var currentState: ConnectionState = ConnectionState.DISCONNECTED

    /**
     * Gets the current connection state.
     */
    fun getState(): ConnectionState = currentState

    /**
     * Transition the state machine based on the given event.
     * Logs every transition.
     */
    @Synchronized
    fun transition(event: ConnectionEvent): ConnectionState {
        val oldState = currentState
        
        // Global override: any disconnect/error resets back to DISCONNECTED
        if (event == ConnectionEvent.DISCONNECT_OR_ERROR || event == ConnectionEvent.AUTH_FAILED) {
            currentState = ConnectionState.DISCONNECTED
            if (oldState != currentState) {
                Log.i(TAG, "[State Transition] $oldState --($event)--> $currentState")
            }
            return currentState
        }

        currentState = matchTransition(oldState, event)

        if (oldState != currentState) {
            Log.i(TAG, "[State Transition] $oldState --($event)--> $currentState")
        }
        return currentState
    }

    private fun matchTransition(current: ConnectionState, event: ConnectionEvent): ConnectionState {
        return when (current) {
            ConnectionState.DISCONNECTED -> when (event) {
                ConnectionEvent.TCP_CONNECTED -> ConnectionState.TCP_CONNECTED
                else -> current
            }
            ConnectionState.TCP_CONNECTED -> when (event) {
                ConnectionEvent.AUTH_SUCCESS -> ConnectionState.AUTHENTICATED
                else -> current
            }
            ConnectionState.AUTHENTICATED -> when (event) {
                ConnectionEvent.ENTER_IDLE -> ConnectionState.IDLE
                else -> current
            }
            ConnectionState.IDLE -> when (event) {
                ConnectionEvent.START_SEND -> ConnectionState.SENDING_IMAGE
                else -> current
            }
            ConnectionState.SENDING_IMAGE -> when (event) {
                ConnectionEvent.SEND_COMPLETE -> ConnectionState.IDLE
                else -> current
            }
        }
    }
}
