"use strict";

const CONFIG = {
    websocketUrl: `ws://${window.location.hostname || "127.0.0.1"}:3001/ws`,
    testEndpoint: "/api/test/follow",
    fallbackAvatar: "/assets/avatar.svg",
    position: {
        top: "32px",
        right: "32px"
    },
    colors: {
        background: "rgba(40, 40, 42, 0.82)",
        text: "#ffffff",
        eventText: "rgba(255, 255, 255, 0.6)",
        followAccent: "#a78bfa"
    },
    animation: {
        displayDuration: 8,
        slideDistance: 1000,
        animationSpeed: 1.2,
        textSpeed: 0.2
    }
};

const elements = {
    alert: document.getElementById("alert"),
    username: document.getElementById("username"),
    event: document.getElementById("event"),
    avatarContainer: document.getElementById("avatarContainer"),
    avatar: document.getElementById("avatar"),
    shine: document.querySelector(".shine"),
    timer: document.getElementById("timer"),
    timerProgress: document.getElementById("timerProgress"),
    tester: document.getElementById("tester"),
    connectionStatus: document.getElementById("connStatus"),
    followButton: document.getElementById("followBtn"),
    connectTwitchButton: document.getElementById("connectTwitchBtn"),
    twitchAuthStatus: document.getElementById("twitchAuthStatus"),
    twitchCodeBlock: document.getElementById("twitchCodeBlock"),
    twitchUserCode: document.getElementById("twitchUserCode"),
    twitchVerifyLink: document.getElementById("twitchVerifyLink")
};

const DEBUG = new URLSearchParams(window.location.search).get("debug") === "1";
const queue = [];
const processedEventIds = new Set();
const TIMER_RADIUS = 15;
const TIMER_CIRCUMFERENCE = 2 * Math.PI * TIMER_RADIUS;
let isPlaying = false;
let socket;
let reconnectDelay = 1000;
let reconnectTimer;

function applyConfig() {
    for (const side of ["top", "right", "bottom", "left"]) {
        elements.alert.style[side] = CONFIG.position[side] || "";
    }

    elements.alert.style.setProperty("--alert-bg", CONFIG.colors.background);
    elements.alert.style.setProperty("--alert-text", CONFIG.colors.text);
    elements.alert.style.setProperty("--event-text", CONFIG.colors.eventText);
    elements.alert.style.setProperty("--accent", CONFIG.colors.followAccent);
    elements.timerProgress.style.strokeDasharray = String(TIMER_CIRCUMFERENCE);
    elements.timerProgress.style.strokeDashoffset = "0";
}

function setConnectionStatus(message, className = "") {
    elements.connectionStatus.textContent = message;
    elements.connectionStatus.className = className;
}

function connectWebSocket() {
    window.clearTimeout(reconnectTimer);
    setConnectionStatus("connecting…");

    socket = new WebSocket(CONFIG.websocketUrl);

    socket.addEventListener("open", () => {
        reconnectDelay = 1000;
        setConnectionStatus("connected to local service", "connected");
    });

    socket.addEventListener("message", (event) => {
        let message;

        try {
            message = JSON.parse(event.data);
        } catch (error) {
            console.error("Ignored invalid WebSocket message", error);
            return;
        }

        if (message.type === "follow") {
            receiveFollow(message);
        }
    });

    socket.addEventListener("close", () => {
        setConnectionStatus("disconnected — retrying…", "error");
        scheduleReconnect();
    });

    socket.addEventListener("error", () => {
        setConnectionStatus("connection error — retrying…", "error");
        socket.close();
    });
}

function scheduleReconnect() {
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(connectWebSocket, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 15000);
}

function receiveFollow(message) {
    if (typeof message.user !== "string" || message.user.trim() === "") {
        console.warn("Ignored follow event without a username", message);
        return;
    }

    if (typeof message.event_id === "string" && message.event_id.length > 0) {
        if (processedEventIds.has(message.event_id)) {
            return;
        }

        processedEventIds.add(message.event_id);
        if (processedEventIds.size > 500) {
            const oldestId = processedEventIds.values().next().value;
            processedEventIds.delete(oldestId);
        }
    }

    enqueue({
        type: "follow",
        user: message.user,
        avatar: typeof message.avatar === "string" && message.avatar.length > 0
            ? message.avatar
            : CONFIG.fallbackAvatar
    });
}

function enqueue(alert) {
    queue.push(alert);
    playNext();
}

function playNext() {
    if (isPlaying) {
        return;
    }

    const nextAlert = queue.shift();
    if (!nextAlert) {
        return;
    }

    isPlaying = true;
    showAlert(nextAlert);
}

function updateContent(alert) {
    elements.username.textContent = alert.user;
    elements.avatar.src = alert.avatar;
    elements.avatar.onerror = () => {
        elements.avatar.onerror = null;
        elements.avatar.src = CONFIG.fallbackAvatar;
    };
    elements.event.textContent = "New Follower";
    elements.alert.dataset.type = "follow";
    elements.alert.style.setProperty("--accent", CONFIG.colors.followAccent);
}

function showAlert(alert) {
    updateContent(alert);

    if (typeof window.gsap === "undefined") {
        showAlertWithoutGsap();
        return;
    }

    playGsapAnimation();
}

function playGsapAnimation() {
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const timeline = window.gsap.timeline({
        onComplete: () => {
            isPlaying = false;
            playNext();
        }
    });

    window.gsap.set(elements.alert, {
        x: reducedMotion ? 0 : CONFIG.animation.slideDistance,
        opacity: 0
    });
    window.gsap.set(elements.avatarContainer, { scale: reducedMotion ? 1 : 0 });
    window.gsap.set([elements.username, elements.event], {
        opacity: 0,
        y: reducedMotion ? 0 : 6
    });
    window.gsap.set(elements.shine, { x: "-150%" });
    window.gsap.set(elements.timer, { opacity: 0 });
    window.gsap.set(elements.timerProgress, {
        strokeDasharray: TIMER_CIRCUMFERENCE,
        strokeDashoffset: 0
    });

    if (reducedMotion) {
        timeline.to(elements.alert, { opacity: 1, duration: 0.1 });
        timeline.to([elements.avatarContainer, elements.username, elements.event, elements.timer], {
            opacity: 1,
            duration: 0.1
        }, "<");
        timeline.to(elements.timerProgress, {
            strokeDashoffset: TIMER_CIRCUMFERENCE,
            duration: CONFIG.animation.displayDuration,
            ease: "none"
        });
        timeline.to(elements.alert, { opacity: 0, duration: 0.1 });
        return;
    }

    timeline.to(elements.alert, {
        x: 0,
        opacity: 1,
        duration: CONFIG.animation.animationSpeed,
        ease: "expo.out"
    });
    timeline.to(elements.avatarContainer, { scale: 2, duration: 0.3 }, "-=0.5");
    timeline.to(elements.avatarContainer, { scale: 1.5, duration: 0.2, ease: "power2.out" });
    timeline.to(elements.timer, { opacity: 1, duration: 0.3 }, "<");
    timeline.to(elements.username, { opacity: 1, y: 0, duration: CONFIG.animation.textSpeed }, "<");
    timeline.to(elements.event, { opacity: 1, y: 0, duration: CONFIG.animation.textSpeed }, "-=0.1");
    timeline.to(elements.shine, { x: "600%", duration: 2.2, ease: "power2.out" }, "<");
    timeline.to(elements.timerProgress, {
        strokeDashoffset: TIMER_CIRCUMFERENCE,
        duration: CONFIG.animation.displayDuration,
        ease: "none"
    }, "<");
    timeline.to(elements.alert, {
        x: CONFIG.animation.slideDistance,
        opacity: 0,
        duration: 0.8,
        ease: "expo.in"
    });
}

function showAlertWithoutGsap() {
    elements.alert.style.transition = "opacity 160ms ease";
    elements.alert.style.opacity = "1";
    elements.username.style.opacity = "1";
    elements.event.style.opacity = "1";
    elements.avatarContainer.style.opacity = "1";
    elements.timer.style.opacity = "1";
    elements.shine.style.display = "none";

    window.setTimeout(() => {
        elements.alert.style.opacity = "0";
        window.setTimeout(() => {
            isPlaying = false;
            playNext();
        }, 180);
    }, CONFIG.animation.displayDuration * 1000);
}

let authPollTimer;

async function requestTwitchAuth() {
    elements.connectTwitchButton.disabled = true;
    elements.twitchAuthStatus.textContent = "Starting Twitch authorization…";

    try {
        const response = await fetch("/api/twitch/auth/start", { method: "POST" });
        const result = await response.json();

        if (!response.ok) {
            throw new Error(result.error || `Request failed with status ${response.status}`);
        }

        renderTwitchAuthStatus(result);
        if (result.state === "authorizing") {
            pollTwitchAuthStatus();
        }
    } catch (error) {
        console.error("Could not start Twitch authorization", error);
        elements.twitchAuthStatus.textContent = error.message || "Could not start Twitch authorization.";
        elements.connectTwitchButton.disabled = false;
    }
}

async function pollTwitchAuthStatus() {
    window.clearTimeout(authPollTimer);

    try {
        const response = await fetch("/api/twitch/auth/status", { cache: "no-store" });
        if (!response.ok) {
            throw new Error(`Status request failed with ${response.status}`);
        }

        const status = await response.json();
        renderTwitchAuthStatus(status);

        if (status.state === "authorizing") {
            authPollTimer = window.setTimeout(pollTwitchAuthStatus, 2000);
        }
    } catch (error) {
        console.error("Could not read Twitch authorization status", error);
        elements.twitchAuthStatus.textContent = "Could not read authorization status. Check the local service.";
        elements.connectTwitchButton.disabled = false;
    }
}

function renderTwitchAuthStatus(status) {
    elements.twitchAuthStatus.textContent = status.message || "Twitch authorization status updated.";
    elements.twitchCodeBlock.hidden = !(status.state === "authorizing" && status.user_code);
    elements.twitchUserCode.textContent = status.user_code || "";
    elements.twitchVerifyLink.href = status.verification_uri || "https://www.twitch.tv/activate";
    elements.connectTwitchButton.disabled = status.state === "authorizing" || status.state === "connected";
    elements.connectTwitchButton.textContent = status.state === "connected" ? "Twitch connected" : "Connect Twitch account";

    if (status.state !== "authorizing") {
        window.clearTimeout(authPollTimer);
    }
}

async function triggerTestFollow() {
    elements.followButton.disabled = true;

    try {
        const response = await fetch(CONFIG.testEndpoint, { method: "POST" });
        const message = await response.text();

        if (!response.ok) {
            throw new Error(message || `Request failed with status ${response.status}`);
        }
    } catch (error) {
        console.error("Could not trigger synthetic follow event", error);
        setConnectionStatus("test failed — check service", "error");
    } finally {
        elements.followButton.disabled = false;
    }
}

applyConfig();
if (DEBUG) {
    elements.tester.classList.add("visible");
}
elements.followButton.addEventListener("click", triggerTestFollow);
elements.connectTwitchButton.addEventListener("click", requestTwitchAuth);
if (DEBUG) {
    pollTwitchAuthStatus();
}
connectWebSocket();
